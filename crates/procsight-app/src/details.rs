//! S1 の詳細パネル(画面設計書 §4.4): 全般・スレッド・DLL・ハンドル・署名の 5 つのタブと、CPU とメモリの小さなグラフ(FR-37)。
//! 取得はタブを開いたときに Job を 1 回送るだけ。触れないプロセスでは何も送らない(INV-1)。
//! ハンドルの表は B の HandleNameWorker と共有するので、描くたびに短く lock して読む(INTERFACES §3.4)。

use eframe::egui;
use crate::app::{Load, State};
use crate::dialogs::Dialog;
use crate::perf::sparkline;
use crate::theme;
use egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::TableBuilder;
use procsight_core::diff::{THREAD_STATE_WAITING, WAIT_REASON_SUSPENDED};
use procsight_core::format;
use procsight_core::ring::Ring;
use procsight_core::{Priority, ProcId, Snapshot};
use procsight_win::jobs::{GeneralInfo, HandleName, Job, ModuleRow, OpError, SharedHandleTable, SigStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DTab {
    #[default]
    General,
    Threads,
    Modules,
    Handles,
    Signature,
}

impl DTab {
    pub const ALL: [DTab; 5] = [DTab::General, DTab::Threads, DTab::Modules, DTab::Handles, DTab::Signature];
    pub fn label(self) -> &'static str {
        match self {
            DTab::General => "全般",
            DTab::Threads => "スレッド",
            DTab::Modules => "DLL",
            DTab::Handles => "ハンドル",
            DTab::Signature => "署名",
        }
    }
}

/// ハンドルの表の種類の一覧と、絞り込んだ行の位置(行の数と選んだ種類が鍵)
#[derive(Default)]
pub struct HandleView {
    len: usize,
    ty: Option<String>,
    types: Vec<String>,
    idx: Vec<usize>,
}

#[derive(Default)]
pub struct DetailState {
    pub id: Option<ProcId>,
    pub tab: DTab,
    pub collapsed: bool,
    pub general: Load<GeneralInfo>,
    pub modules: Load<Vec<ModuleRow>>,
    pub module_filter: String,
    pub handles: Load<SharedHandleTable>,
    pub handle_type: Option<String>,
    pub handle_view: Option<HandleView>,
    /// FR-37。選び直したら空から
    pub cpu: Ring<f32>,
    pub mem: Ring<f32>,
}

impl DetailState {
    pub fn reset(&mut self, id: ProcId) {
        if self.id == Some(id) {
            return;
        }
        self.id = Some(id);
        self.general = Load::NotAsked;
        self.modules = Load::NotAsked;
        self.handles = Load::NotAsked;
        self.handle_type = None;
        self.handle_view = None;
        self.module_filter.clear();
        self.cpu.clear();
        self.mem.clear();
    }

    pub fn push_history(&mut self, snap: &Snapshot) {
        let Some(r) = self.id.and_then(|id| snap.find(id)) else { return };
        if r.is_gone > 0 || r.touch_free {
            return;
        }
        self.cpu.push(r.cpu_pct.unwrap_or(0.0));
        self.mem.push(r.mem.unwrap_or(0) as f32);
    }
}

pub fn on_general(st: &mut State, id: ProcId, info: Result<GeneralInfo, OpError>) {
    let d = &mut st.procs.detail;
    if d.id == Some(id) {
        d.general = match info {
            Ok(i) => Load::Ready(i),
            Err(e) => Load::Failed(e),
        };
    }
}

pub fn on_modules(st: &mut State, id: ProcId, r: Result<Vec<ModuleRow>, OpError>) {
    let d = &mut st.procs.detail;
    if d.id == Some(id) {
        d.modules = match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        };
    }
}

pub fn on_handles(st: &mut State, id: ProcId, r: Result<SharedHandleTable, OpError>) {
    let d = &mut st.procs.detail;
    if d.id == Some(id) {
        d.handle_view = None;
        d.handles = match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        };
    }
}

/// 効率モードの今の状態(読めたときだけ。FR-14)
pub fn efficiency_of(st: &State, id: ProcId) -> Option<bool> {
    let d = &st.procs.detail;
    match (&d.general, d.id == Some(id)) {
        (Load::Ready(g), true) => g.efficiency,
        _ => None,
    }
}

/// 効率モードを変えたら全般を取り直す
pub fn forget_general(st: &mut State, id: ProcId) {
    if st.procs.detail.id == Some(id) {
        st.procs.detail.general = Load::NotAsked;
    }
}

fn loading(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new());
        ui.label("取得しています");
    });
}

fn failed(st: &mut State, ui: &mut Ui, e: &OpError) {
    let pal = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("取得できません({})", e.message(st.env.elevated))).color(pal.danger));
        if e.is_access_denied() && !st.env.elevated && ui.button("管理者で再起動").clicked() {
            st.dialog = Some(Dialog::RestartElevated { opened: false });
        }
    });
}

fn value_text<T>(r: &Result<T, OpError>, elevated: bool, f: impl Fn(&T) -> String) -> String {
    match r {
        Ok(v) => f(v),
        Err(e) => format!("取得できません({})", e.message(elevated)),
    }
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    let id = st.procs.single();
    if let Some(id) = id {
        st.procs.detail.reset(id);
    }
    let row = id.and_then(|i| st.snap.find(i)).cloned();
    ui.horizontal(|ui| {
        match &row {
            Some(r) => ui.label(RichText::new(format!("詳細: {}(PID {})", r.name, r.id.pid)).strong()),
            None => ui.label(RichText::new("詳細").strong()),
        };
        ui.add_space(12.0);
        for t in DTab::ALL {
            if ui.selectable_label(st.procs.detail.tab == t, t.label()).clicked() {
                st.procs.detail.tab = t;
                st.procs.detail.collapsed = false;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (label, tip) = if st.procs.detail.collapsed { ("∨ 開く", "詳細を開く") } else { ("∧ 畳む", "見出しの行だけにする") };
            if ui.button(label).on_hover_text(tip).clicked() {
                st.procs.detail.collapsed = !st.procs.detail.collapsed;
            }
        });
    });
    if st.procs.detail.collapsed {
        return;
    }
    // 中身が少ないタブでもパネルの高さを保つ(ドラッグで変えた高さを、タブを替えても変えない)
    ui.set_min_height(ui.available_height());
    ui.separator();
    let Some(r) = row else {
        ui.label(RichText::new("行を選ぶと、ここに詳細が出ます").color(pal.weak));
        return;
    };
    if r.touch_free {
        ui.label(RichText::new("触れないプロセスのため、詳細は取得しません").color(pal.weak));
        return;
    }
    if r.is_gone > 0 {
        ui.label(RichText::new("すでに終了しています").color(pal.weak));
        return;
    }
    match st.procs.detail.tab {
        DTab::General => general(st, ui, &r),
        DTab::Threads => threads(st, ui, &r),
        DTab::Modules => modules(st, ui, r.id),
        DTab::Handles => handles(st, ui, r.id),
        DTab::Signature => signature(st, ui, &r),
    }
}

fn copy_menu(resp: &egui::Response, text: &str) {
    resp.context_menu(|ui| {
        if ui.button("コピー").clicked() {
            ui.ctx().copy_text(text.to_string());
            ui.close();
        }
    });
}

fn general(st: &mut State, ui: &mut Ui, r: &procsight_core::ProcessRow) {
    if st.procs.detail.general.is_not_asked() {
        st.procs.detail.general = Load::Loading;
        st.job(Job::General { id: r.id });
    }
    let pal = theme::palette(ui);
    let info = match &st.procs.detail.general {
        Load::Ready(g) => Some(g.clone()),
        Load::Failed(e) => {
            let e = e.clone();
            failed(st, ui, &e);
            return;
        }
        _ => None,
    };
    let el = st.env.elevated;
    let mut jump = None;
    ui.horizontal_top(|ui| {
        let graph_w = 220.0;
        let left_w = (ui.available_width() - graph_w - 12.0).max(200.0);
        let h = ui.available_height();
        ui.allocate_ui_with_layout(egui::vec2(left_w, h), Layout::top_down(Align::Min), |ui| {
            ui.set_min_size(egui::vec2(left_w, h));
            egui::ScrollArea::vertical().id_salt("general").auto_shrink([false, false]).show(ui, |ui| {
                let Some(g) = info else {
                    loading(ui);
                    return;
                };
                let mut rows: Vec<(&str, String, bool)> = vec![
                    ("パス", value_text(&g.path, el, |s| s.clone()), true),
                    ("コマンドライン", value_text(&g.command_line, el, |s| s.clone()), true),
                ];
                let parent = match r.parent.and_then(|p| st.snap.find(p)) {
                    Some(p) => format!("{}(PID {})", p.name, p.id.pid),
                    None if r.parent_pid != 0 => format!("{}(終了)", r.parent_pid),
                    None => format::DASH.into(),
                };
                rows.push(("親", parent, false));
                rows.push(("ユーザー", value_text(&g.user, el, |s| s.clone()), false));
                rows.push(("昇格", value_text(&g.elevated, el, |b| if *b { "はい" } else { "いいえ" }.into()), false));
                rows.push(("整合性レベル", value_text(&g.integrity, el, |s| s.clone()), false));
                rows.push(("アーキテクチャ", value_text(&g.arch, el, |a| a.label()), false));
                rows.push(("開始時刻", format::filetime(r.id.create_time, st.env.utc_offset_minutes), false));
                rows.push(("CPU 時間", value_text(&g.cpu_time, el, |t| format::cpu_time(*t)), false));
                rows.push(("優先度", Priority::base_label(r.base_priority), false));
                if let Some(e) = g.efficiency {
                    rows.push(("効率モード", if e { "オン" } else { "オフ" }.into(), false));
                }
                rows.push(("セッション", r.session_id.to_string(), false));
                // 項目・値・[コピー] の 3 列。値の列は残りの幅いっぱい
                let key_w = 120.0;
                let btn_w = 64.0;
                for (k, v, copy) in &rows {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(egui::vec2(key_w, 18.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(key_w);
                            ui.label(RichText::new(*k).color(pal.weak));
                        });
                        let val_w = (ui.available_width() - btn_w).max(80.0);
                        let resp = ui
                            .allocate_ui_with_layout(egui::vec2(val_w, 18.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_min_width(val_w);
                                if *k == "親" && r.parent.is_some() {
                                    let l = ui.link(v);
                                    if l.clicked() {
                                        jump = r.parent;
                                    }
                                    l
                                } else {
                                    ui.add(egui::Label::new(v.as_str()).truncate().sense(Sense::click()))
                                }
                            })
                            .inner;
                        copy_menu(&resp, v);
                        if *copy && !v.starts_with("取得できません") && ui.small_button("コピー").clicked() {
                            ui.ctx().copy_text(v.clone());
                        }
                    });
                }
            });
        });
        ui.vertical(|ui| {
            let d = &st.procs.detail;
            ui.label(RichText::new(format!("CPU {}", format::pct(d.cpu.last().copied()))).color(pal.weak));
            sparkline(ui, d.cpu.iter().copied(), 100.0, egui::vec2(graph_w, 56.0), pal.graph);
            ui.add_space(6.0);
            let mem_max = d.mem.iter().copied().fold(1.0f32, f32::max) * 1.2;
            ui.label(RichText::new(format!("メモリ {}", format::opt_bytes(d.mem.last().map(|v| *v as u64)))).color(pal.weak));
            sparkline(ui, d.mem.iter().copied(), mem_max, egui::vec2(graph_w, 56.0), pal.graph2);
        });
    });
    if let Some(p) = jump {
        st.procs.scroll_to = Some(p);
    }
}

/// D-20: 開始アドレスに DLL の名前を添える(DLL の一覧を取ってあれば)
fn address_text(modules: &Load<Vec<ModuleRow>>, addr: u64) -> String {
    if let Load::Ready(ms) = modules
        && let Some(m) = ms.iter().find(|m| addr >= m.base && addr < m.base + m.size as u64)
    {
        return format!("{}+0x{:X}", m.name, addr - m.base);
    }
    format!("0x{addr:016X}")
}

fn threads(st: &mut State, ui: &mut Ui, r: &procsight_core::ProcessRow) {
    let pal = theme::palette(ui);
    // 更新ごとに SPI から取り直す(ハンドルを開かない。D-20)
    let want = (r.id, st.snap.seq);
    if st.threads_req != Some(want) {
        st.threads_req = Some(want);
        st.job(Job::Threads { id: r.id });
    }
    let Some(list) = st.threads.get(&r.id) else {
        ui.label(RichText::new(format!("スレッド {} 本。取得しています", format::thousands(r.threads as u64))).color(pal.weak));
        return;
    };
    let modules = &st.procs.detail.modules;
    TableBuilder::new(ui)
        .id_salt("threads")
        .striped(true)
        .resizable(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(egui_extras::Column::initial(90.0))
        .column(egui_extras::Column::initial(80.0))
        .column(egui_extras::Column::initial(100.0))
        .column(egui_extras::Column::initial(70.0))
        .column(egui_extras::Column::initial(90.0))
        .column(egui_extras::Column::remainder())
        .header(20.0, |mut h| {
            for t in ["スレッド ID", "状態", "待ちの理由", "優先度", "CPU 時間", "開始アドレス"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(18.0, list.len(), |mut row| {
                let t = &list[row.index()];
                // §8.1 の表が無いので、core で確かめると決めた値だけ言葉にし、他は数字のまま(FR-31)
                let state = if t.state == THREAD_STATE_WAITING { "待ち".to_string() } else { t.state.to_string() };
                let wait = if t.state != THREAD_STATE_WAITING {
                    format::DASH.to_string()
                } else if t.wait_reason == WAIT_REASON_SUSPENDED {
                    "一時停止".into()
                } else {
                    t.wait_reason.to_string()
                };
                let cpu = match (t.kernel_time, t.user_time) {
                    (Some(k), Some(u)) => format::cpu_time(k + u),
                    _ => format::DASH.into(),
                };
                let cells = [t.tid.to_string(), state, wait, t.priority.to_string(), cpu, address_text(modules, t.start_address)];
                for c in cells {
                    row.col(|ui| {
                        ui.label(c);
                    });
                }
            });
        });
}

fn modules(st: &mut State, ui: &mut Ui, id: ProcId) {
    if st.procs.detail.modules.is_not_asked() {
        st.procs.detail.modules = Load::Loading;
        st.job(Job::Modules { id });
    }
    let mut again = false;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.procs.detail.module_filter).hint_text("名前・パスで絞り込み").desired_width(220.0));
        if ui.button("取り直す").clicked() {
            again = true;
        }
        if let Load::Ready(v) = &st.procs.detail.modules {
            ui.label(format!("{} 件", format::thousands(v.len() as u64)));
        }
    });
    if again {
        st.procs.detail.modules = Load::Loading;
        st.job(Job::Modules { id });
    }
    let list = match &st.procs.detail.modules {
        Load::Ready(v) => v,
        Load::Failed(e) => {
            let e = e.clone();
            failed(st, ui, &e);
            return;
        }
        _ => {
            loading(ui);
            return;
        }
    };
    if list.is_empty() {
        ui.label("ありません");
        return;
    }
    let q = st.procs.detail.module_filter.trim().to_lowercase();
    let idx: Vec<usize> = (0..list.len())
        .filter(|&i| q.is_empty() || list[i].name.to_lowercase().contains(&q) || list[i].path.to_lowercase().contains(&q))
        .collect();
    TableBuilder::new(ui)
        .id_salt("modules")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(egui_extras::Column::initial(160.0))
        .column(egui_extras::Column::initial(360.0).clip(true))
        .column(egui_extras::Column::initial(150.0))
        .column(egui_extras::Column::remainder())
        .header(20.0, |mut h| {
            for t in ["名前", "パス", "ベースアドレス", "サイズ"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(18.0, idx.len(), |mut row| {
                let m = &list[idx[row.index()]];
                let cells = [m.name.clone(), m.path.clone(), format!("0x{:016X}", m.base), format::bytes(m.size as u64)];
                for c in &cells {
                    row.col(|ui| {
                        ui.add(egui::Label::new(c.as_str()).truncate());
                    });
                }
                let line = cells.join("\t");
                copy_menu(&row.response(), &line);
            });
        });
}

fn handle_name(n: &HandleName) -> String {
    match n {
        HandleName::Pending => "取得しています".into(),
        HandleName::None => String::new(),
        HandleName::Name(s) => s.clone(),
        HandleName::TimedOut => "取得できません(応答なし)".into(),
        HandleName::Skipped => "取得していません".into(),
        HandleName::Failed(c) => format!("取得できません(エラー {c})"),
    }
}

fn handles(st: &mut State, ui: &mut Ui, id: ProcId) {
    if st.procs.detail.handles.is_not_asked() {
        st.procs.detail.handles = Load::Loading;
        st.job(Job::Handles { id });
    }
    let table = match &st.procs.detail.handles {
        Load::Ready(t) => t.clone(),
        Load::Failed(e) => {
            let e = e.clone();
            failed(st, ui, &e);
            return;
        }
        _ => {
            loading(ui);
            return;
        }
    };
    let Ok(t) = table.lock() else {
        ui.label("取得できません");
        return;
    };
    if !t.finished {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    // 種類の一覧と絞り込みの位置は、行の数か種類の選択が変わったときだけ作る(5 万行で lock を長く持たない。§3.4)
    let d = &mut st.procs.detail;
    let fresh = d.handle_view.as_ref().is_some_and(|v| v.len == t.rows.len() && v.ty == d.handle_type);
    if !fresh {
        let mut types: Vec<String> = t.rows.iter().map(|r| r.type_name.clone()).collect();
        types.sort_unstable();
        types.dedup();
        let ty = d.handle_type.clone();
        let idx = (0..t.rows.len()).filter(|&i| ty.as_deref().is_none_or(|x| t.rows[i].type_name == x)).collect();
        d.handle_view = Some(HandleView { len: t.rows.len(), ty, types, idx });
    }
    let view = d.handle_view.take().unwrap_or_default();
    let types = &view.types;
    let mut again = false;
    let sel = &mut st.procs.detail.handle_type;
    ui.horizontal(|ui| {
        let l = ui.label("種類");
        egui::ComboBox::from_id_salt("htype")
            .selected_text(sel.clone().unwrap_or_else(|| "すべての種類".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(sel, None, "すべての種類");
                for ty in types {
                    ui.selectable_value(sel, Some(ty.clone()), ty.as_str());
                }
            })
            .response
            .labelled_by(l.id);
        if ui.button("取り直す").clicked() {
            again = true;
        }
        let n = format::thousands(t.rows.len() as u64);
        if t.finished || t.names_total == 0 {
            ui.label(format!("{n} 件"));
        } else {
            let pct = t.names_done * 100 / t.names_total.max(1);
            ui.label(format!("{n} 件(名前の取得中 {pct}%)"));
        }
    });
    let idx = &view.idx;
    if t.rows.is_empty() {
        ui.label("ありません");
    } else {
        TableBuilder::new(ui)
            .id_salt("handles")
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(egui_extras::Column::initial(90.0))
            .column(egui_extras::Column::initial(140.0))
            .column(egui_extras::Column::initial(420.0).clip(true))
            .column(egui_extras::Column::remainder())
            .header(20.0, |mut h| {
                for s in ["値", "種類", "名前", "アクセス"] {
                    h.col(|ui| {
                        ui.strong(s);
                    });
                }
            })
            .body(|body| {
                body.rows(18.0, idx.len(), |mut row| {
                    let h = &t.rows[idx[row.index()]];
                    let cells = [format!("0x{:X}", h.value), h.type_name.clone(), handle_name(&h.name), format!("0x{:08X}", h.access)];
                    for c in &cells {
                        row.col(|ui| {
                            ui.add(egui::Label::new(c.as_str()).truncate());
                        });
                    }
                    copy_menu(&row.response(), &cells.join("\t"));
                });
            });
    }
    drop(t);
    st.procs.detail.handle_view = Some(view);
    if again {
        st.procs.detail.handles = Load::Loading;
        st.job(Job::Handles { id });
    }
}

fn signature(st: &mut State, ui: &mut Ui, r: &procsight_core::ProcessRow) {
    let pal = theme::palette(ui);
    let Some(path) = r.path().map(str::to_string) else {
        ui.label(RichText::new("パスを取得できていないため、確かめられません").color(pal.weak));
        return;
    };
    if !st.sigs.contains_key(&path) {
        st.sigs.insert(path.clone(), None);
        st.job(Job::Signature { path: path.clone() });
    }
    match st.sigs.get(&path) {
        Some(Some(s)) => {
            let method = match s.status {
                SigStatus::Embedded => "ファイルに埋め込まれた署名",
                SigStatus::Catalog => "Windows のカタログ",
                _ => format::DASH,
            };
            let color = match s.status {
                SigStatus::Embedded | SigStatus::Catalog => ui.visuals().text_color(),
                SigStatus::Unsigned => pal.warn,
                SigStatus::Failed(_) => pal.danger,
            };
            egui::Grid::new("sig").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new("状態").color(pal.weak));
                ui.label(RichText::new(s.status.label()).color(color));
                ui.end_row();
                ui.label(RichText::new("署名者").color(pal.weak));
                ui.label(s.signer.clone().unwrap_or_else(|| format::DASH.into()));
                ui.end_row();
                ui.label(RichText::new("確かめた方法").color(pal.weak));
                ui.label(method);
                ui.end_row();
                ui.label(RichText::new("確かめた時刻").color(pal.weak));
                ui.label(format::filetime(s.checked_at, st.env.utc_offset_minutes));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new("ネットには問い合わせていません").color(pal.weak));
        }
        _ => loading(ui),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_address_uses_module_names() {
        let ms = Load::Ready(vec![ModuleRow { name: "ntdll.dll".into(), path: String::new(), base: 0x1000, size: 0x100 }]);
        assert_eq!(address_text(&ms, 0x1010), "ntdll.dll+0x10");
        assert_eq!(address_text(&ms, 0x2000), "0x0000000000002000");
        assert_eq!(handle_name(&HandleName::TimedOut), "取得できません(応答なし)");
    }
}
