//! S1 の詳細パネル(画面設計書 §4.4): 全般・スレッド・DLL・ハンドル・署名の 5 つのタブと、CPU とメモリの小さなグラフ(FR-37)。
//! 取得はタブを開いたときに Job を 1 回送るだけ。触れないプロセスでは何も送らない(INV-1)。
//! ハンドルの表は B の HandleNameWorker と共有するので、描くたびに短く lock して読む(INTERFACES §3.4)。

use eframe::egui;
use crate::app::{Load, State};
use crate::dialogs::Dialog;
use crate::perf::sparkline;
use crate::theme::{self, size, space};
use crate::widgets;
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

/// 取得中: くるくる回る印と「取得しています」を真ん中に
fn loading(ui: &mut Ui) {
    ui.add_space(space::XL);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new());
        ui.add_space(space::S);
        ui.label(RichText::new("取得しています").color(theme::palette(ui).weak));
    });
}

/// 取れない: 「取得できません(理由)」。権限が理由なら「管理者で再起動」
fn failed(st: &mut State, ui: &mut Ui, e: &OpError) {
    let pal = theme::palette(ui);
    ui.add_space(space::XL);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(format!("取得できません({})", e.message(st.env.elevated))).color(pal.danger));
        if e.is_access_denied() && !st.env.elevated {
            ui.add_space(space::M);
            if ui.add(widgets::primary_button(&pal, "管理者で再起動")).clicked() {
                st.dialog = Some(Dialog::RestartElevated { opened: false });
            }
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
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::M;
        // 畳む・開くは右端に先に置き、左の見出しとタブは残りの幅に収める
        let (label, tip) = if st.procs.detail.collapsed { ("∨ 開く", "詳細を開く") } else { ("∧ 畳む", "見出しの行だけにする") };
        let right_w = 72.0;
        let left_w = (ui.available_width() - right_w).max(0.0);
        ui.allocate_ui_with_layout(egui::vec2(left_w, 30.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            match &row {
                Some(r) => {
                    crate::procs::row_icon(st, ui, r, &pal);
                    // 長い名前でも札とタブを押し出さないよう、名前は札とタブの分を残した幅で切る
                    let name_max = (ui.available_width() - 440.0).max(80.0);
                    // 切るのは名前だけにし、PID は必ず出す
                    ui.scope(|ui| {
                        ui.set_max_width(name_max);
                        let name = RichText::new(r.name.as_str()).size(size::BODY + 1.0).color(ui.visuals().strong_text_color());
                        ui.add(egui::Label::new(name).selectable(false).truncate());
                    });
                    ui.add(egui::Label::new(RichText::new(format!("PID {}", r.id.pid)).size(size::SMALL + 1.0).color(pal.weak)).selectable(false));
                    let state = if r.is_gone > 0 { ("終了", pal.danger) } else { (r.state.label(), crate::procs::state_color(&pal, r.state)) };
                    // 実行中は弱い色だと札が沈むので、成功の色にする
                    let color = if r.state == procsight_core::ProcState::Running && r.is_gone == 0 { pal.ok } else { state.1 };
                    widgets::pill(ui, state.0, color);
                }
                None => {
                    ui.label(RichText::new("詳細").strong());
                }
            }
            ui.add_space(space::M);
            ui.spacing_mut().item_spacing.x = 0.0;
            for t in DTab::ALL {
                if widgets::nav_tab(ui, &pal, st.procs.detail.tab == t, t.label(), None).clicked() {
                    st.procs.detail.tab = t;
                    st.procs.detail.collapsed = false;
                }
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
    let sep = ui.available_rect_before_wrap();
    ui.painter().hline(sep.x_range(), sep.top(), egui::Stroke::new(1.0, pal.border));
    ui.add_space(space::M);
    let Some(r) = row else {
        widgets::empty_state(ui, &pal, "行を選ぶと、ここに詳細が出ます", None);
        return;
    };
    if r.touch_free {
        // §4.4: この文だけを出す
        widgets::empty_state(ui, &pal, "触れないプロセスのため、詳細は取得しません", None);
        return;
    }
    if r.is_gone > 0 {
        widgets::empty_state(ui, &pal, "すでに終了しています", None);
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
    let row_h = st.cfg.density.row_h();
    ui.horizontal_top(|ui| {
        let graph_w = (ui.available_width() * 0.36).clamp(200.0, 440.0);
        let left_w = (ui.available_width() - graph_w - space::XL).max(200.0);
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
                // 項目(弱い文字)・値・[コピー] の 3 列。値の列は残りの幅いっぱい
                let key_w = 116.0;
                let btn_w = 64.0;
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, v, copy) in &rows {
                    ui.horizontal(|ui| {
                        ui.set_min_height(row_h);
                        ui.allocate_ui_with_layout(egui::vec2(key_w, row_h), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(key_w);
                            ui.label(RichText::new(*k).color(pal.weak));
                        });
                        let val_w = (ui.available_width() - btn_w).max(80.0);
                        let failed = v.starts_with("取得できません");
                        let resp = ui
                            .allocate_ui_with_layout(egui::vec2(val_w, row_h), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_min_width(val_w);
                                if *k == "親" && r.parent.is_some() {
                                    let l = ui.link(v);
                                    if l.clicked() {
                                        jump = r.parent;
                                    }
                                    l
                                } else {
                                    let color = if failed { pal.weak } else { ui.visuals().strong_text_color() };
                                    ui.add(egui::Label::new(RichText::new(v.as_str()).color(color)).truncate().sense(Sense::click()))
                                }
                            })
                            .inner;
                        copy_menu(&resp, v);
                        if *copy && !failed && ui.small_button("コピー").clicked() {
                            ui.ctx().copy_text(v.clone());
                        }
                    });
                }
            });
        });
        ui.add_space(space::M);
        ui.vertical(|ui| {
            let d = &st.procs.detail;
            // 2 つのグラフで高さを分ける(見出しの行 2 つ・間・部品の間を引いた残り)。
            // h を超えるとパネルが伸び、伸びた h でまたグラフが伸びるので、必ず h の内に収める
            let sy = ui.spacing().item_spacing.y;
            let gh = ((h - 2.0 * 26.0 - space::L - 4.0 * sy) / 2.0 - 1.0).floor().min(160.0);
            let cpu_now = format::pct(d.cpu.last().copied());
            let mem_now = format::opt_bytes(d.mem.last().map(|v| *v as u64));
            if gh < 32.0 {
                // パネルが低いときはグラフを省き、今の値だけ(パネルを押し広げて表を潰さない)
                graph_title(ui, &pal, "CPU", &cpu_now, graph_w, pal.graph);
                graph_title(ui, &pal, "メモリ", &mem_now, graph_w, pal.graph2);
            } else {
                graph(ui, &pal, "CPU", &cpu_now, d.cpu.iter().copied(), 100.0, egui::vec2(graph_w, gh), pal.graph);
                ui.add_space(space::L);
                let mem_max = d.mem.iter().copied().fold(1.0f32, f32::max) * 1.2;
                graph(ui, &pal, "メモリ", &mem_now, d.mem.iter().copied(), mem_max, egui::vec2(graph_w, gh), pal.graph2);
            }
        });
    });
    if let Some(p) = jump {
        st.procs.scroll_to = Some(p);
    }
}

/// スレッド・DLL・ハンドルの表。パネルの残りの高さで送り、幅いっぱいに広げる(行が多くてもパネルを押し広げない)。
fn detail_table(ui: &mut Ui) -> TableBuilder<'_> {
    let h = ui.available_height();
    TableBuilder::new(ui).min_scrolled_height(0.0).max_scroll_height(h).auto_shrink([false, false])
}

/// グラフの見出しの行: 名前(弱い文字)と今の値(大きく、単位つき)
fn graph_title(ui: &mut Ui, pal: &theme::Palette, name: &str, now: &str, w: f32, color: egui::Color32) {
    ui.allocate_ui_with_layout(egui::vec2(w, 26.0), Layout::left_to_right(Align::Max), |ui| {
        ui.label(RichText::new(name).color(pal.weak));
        ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
            ui.label(RichText::new(now).size(size::TITLE).color(color));
        });
    });
}

/// 全般の右のグラフ 1 つ(FR-37)
#[allow(clippy::too_many_arguments)]
fn graph(ui: &mut Ui, pal: &theme::Palette, name: &str, now: &str, values: impl Iterator<Item = f32>, max: f32, size_: egui::Vec2, color: egui::Color32) {
    graph_title(ui, pal, name, now, size_.x, color);
    sparkline(ui, values, max, size_, color);
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
    let row_h = st.cfg.density.row_h();
    // 更新ごとに SPI から取り直す(ハンドルを開かない。D-20)
    let want = (r.id, st.snap.seq);
    if st.threads_req != Some(want) {
        st.threads_req = Some(want);
        st.job(Job::Threads { id: r.id });
    }
    let Some(list) = st.threads.get(&r.id) else {
        loading(ui);
        return;
    };
    let modules = &st.procs.detail.modules;
    detail_table(ui)
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
        .header(row_h + 4.0, |mut h| {
            for t in ["スレッド ID", "状態", "待ちの理由", "優先度", "CPU 時間", "開始アドレス"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, list.len(), |mut row| {
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
    let row_h = st.cfg.density.row_h();
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
        widgets::empty_state(ui, &theme::palette(ui), "ありません", None);
        return;
    }
    let q = st.procs.detail.module_filter.trim().to_lowercase();
    let idx: Vec<usize> = (0..list.len())
        .filter(|&i| q.is_empty() || list[i].name.to_lowercase().contains(&q) || list[i].path.to_lowercase().contains(&q))
        .collect();
    detail_table(ui)
        .id_salt("modules")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(egui_extras::Column::initial(160.0))
        .column(egui_extras::Column::initial(360.0).clip(true))
        .column(egui_extras::Column::initial(150.0))
        .column(egui_extras::Column::remainder())
        .header(row_h + 4.0, |mut h| {
            for t in ["名前", "パス", "ベースアドレス", "サイズ"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, idx.len(), |mut row| {
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
    let row_h = st.cfg.density.row_h();
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
        widgets::empty_state(ui, &theme::palette(ui), "取得できません", None);
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
        widgets::empty_state(ui, &theme::palette(ui), "ありません", None);
    } else {
        detail_table(ui)
            .id_salt("handles")
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(egui_extras::Column::initial(90.0))
            .column(egui_extras::Column::initial(140.0))
            .column(egui_extras::Column::initial(420.0).clip(true))
            .column(egui_extras::Column::remainder())
            .header(row_h + 4.0, |mut h| {
                for s in ["値", "種類", "名前", "アクセス"] {
                    h.col(|ui| {
                        ui.strong(s);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, idx.len(), |mut row| {
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
        widgets::empty_state(ui, &pal, "パスを取得できていないため、確かめられません", None);
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
