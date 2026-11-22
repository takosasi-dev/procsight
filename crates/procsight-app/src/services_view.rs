//! S4 サービス(画面設計書 §6.2・FR-41〜FR-44)。絞り込み・状態の範囲・開始・停止・再起動・開始の種類・プロセスへ移動。
//! 停止と再起動は依存の一覧を取ってから D5、開始の種類は D6 を通す(INV-5)。昇格していなければ操作を灰色にする。
//! 操作の間は行に「処理中…」を出す(30 秒の打ち切りは Worker。E-31)。状態は色の点(実行中・停止・途中)。

use eframe::egui;
use crate::app::{Load, State, Tab};
use crate::dialogs::Dialog;
use crate::theme::{self, Palette, space};
use crate::widgets;
use egui::{Align, Color32, CornerRadius, Layout, Margin, Sense, Stroke, Ui};
use egui_extras::TableBuilder;
use procsight_core::format;
use procsight_win::jobs::{Job, OpError, ServiceAction, ServiceRow, ServiceState, StartType};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StateFilter {
    #[default]
    All,
    Running,
    Stopped,
}

#[derive(Default)]
pub struct ServicesView {
    pub list: Load<Vec<ServiceRow>>,
    pub filter: String,
    pub state_filter: StateFilter,
    pub selected: Option<String>,
    /// 処理中のサービス(名前 → 表示名)。結果(ServiceDone)には表示名しか無いので、両方を覚える
    pub pending: HashMap<String, String>,
    pub focus_filter: bool,
    /// 次に描くときにこの名前の行まで表を送る(コマンド パレットから移ったとき)
    pub scroll_to: Option<String>,
}

pub fn mark_pending(st: &mut State, name: &str, display_name: &str) {
    st.services.pending.insert(name.to_string(), display_name.to_string());
}

pub fn on_loaded(st: &mut State, r: Result<Vec<ServiceRow>, OpError>) {
    st.services.list = match r {
        Ok(v) => Load::Ready(v),
        Err(e) => Load::Failed(e),
    };
}

fn clear_pending_by_display(st: &mut State, display_name: &str) {
    st.services.pending.retain(|_, d| d != display_name);
}

pub fn on_done(st: &mut State, display_name: String, action: ServiceAction, r: Result<(), OpError>) {
    clear_pending_by_display(st, &display_name);
    match r {
        Ok(()) => {
            let verb = match action {
                ServiceAction::Start => "開始しました",
                ServiceAction::Stop => "停止しました",
                ServiceAction::Restart => "再起動しました",
            };
            st.ok(format!("『{display_name}』を{verb}"));
        }
        Err(e) => st.fail(&e),
    }
    crate::log::info("service", &format!("action={action:?} result=done"));
    st.job_load_services();
}

pub fn on_start_type(st: &mut State, display_name: String, t: StartType, r: Result<(), OpError>) {
    clear_pending_by_display(st, &display_name);
    match r {
        Ok(()) => st.ok(format!("『{display_name}』の開始の種類を『{}』にしました", t.label())),
        Err(e) => st.fail(&e),
    }
    st.job_load_services();
}

pub(crate) fn button(ui: &mut Ui, label: &str, why: Option<&str>) -> bool {
    let r = ui.add_enabled(why.is_none(), egui::Button::new(label));
    match why {
        Some(w) => r.on_disabled_hover_text(w).clicked(),
        None => r.clicked(),
    }
}

/// 枠で囲んだ切り替えの並び(「すべて / 実行中 / 停止」など)
pub(crate) fn segmented<T: PartialEq + Copy>(ui: &mut Ui, pal: &Palette, value: &mut T, items: &[(T, String)]) {
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (v, label) in items {
                    ui.selectable_value(value, *v, label.as_str());
                }
            });
        });
}

/// 読み込み中の表示(回る印と文。empty_state と同じ位置)
pub(crate) fn loading(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.add_space(space::XL * 2.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().color(pal.weak));
        ui.add_space(space::S);
        ui.label(text);
    });
}

/// 状態の色(実行中は成功の色、停止と一時停止は弱い色、開始中・停止中などの途中の物は警告の色。画面設計書 §6.2)
pub fn state_color(pal: &Palette, s: ServiceState) -> Color32 {
    match s {
        ServiceState::Running => pal.ok,
        ServiceState::Stopped | ServiceState::Paused => pal.weak,
        _ => pal.warn,
    }
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    if st.services.list.is_not_asked() {
        st.job_load_services();
    }
    let pal = theme::palette(ui);
    let sel = match (&st.services.list, &st.services.selected) {
        (Load::Ready(v), Some(n)) => v.iter().find(|s| &s.name == n).cloned(),
        _ => None,
    };
    let busy = sel.as_ref().is_some_and(|s| st.services.pending.contains_key(&s.name));
    let counts = match &st.services.list {
        Load::Ready(v) => Some((
            v.len(),
            v.iter().filter(|s| s.state == ServiceState::Running).count(),
            v.iter().filter(|s| s.state == ServiceState::Stopped).count(),
        )),
        _ => None,
    };
    let with_count = |label: &str, n: Option<usize>| match n {
        Some(n) => format!("{label} {n}"),
        None => label.to_string(),
    };
    ui.horizontal_wrapped(|ui| {
        let resp = widgets::search_box(ui, &pal, &mut st.services.filter, "名前・表示名で絞り込み(Ctrl+F)", 240.0);
        if std::mem::take(&mut st.services.focus_filter) {
            resp.request_focus();
        }
        segmented(ui, &pal, &mut st.services.state_filter, &[
            (StateFilter::All, with_count("すべて", counts.map(|c| c.0))),
            (StateFilter::Running, with_count("実行中", counts.map(|c| c.1))),
            (StateFilter::Stopped, with_count("停止", counts.map(|c| c.2))),
        ]);
        ui.add_space(space::M);
        let none = sel.is_none().then_some("サービスを選んでください");
        let admin = (!st.env.elevated).then_some("管理者で再起動すると変えられます");
        let busy_why = busy.then_some("処理中です");
        let base = none.or(admin).or(busy_why);
        let can_start = base.or(sel.as_ref().filter(|s| s.state != ServiceState::Stopped).map(|_| "停止しているときだけ開始できます"));
        let can_stop = base
            .or(sel.as_ref().filter(|s| s.state != ServiceState::Running).map(|_| "実行中のときだけ停止できます"))
            .or(sel.as_ref().filter(|s| !s.accepts_stop).map(|_| "このサービスは停止を受け付けません"));
        if button(ui, "開始", can_start) {
            let s = sel.clone().unwrap();
            mark_pending(st, &s.name, &s.display_name);
            st.job(Job::ServiceControl {
                name: s.name,
                display_name: s.display_name,
                action: ServiceAction::Start,
                confirmed: Vec::new(),
            });
        }
        for (label, action) in [("停止", ServiceAction::Stop), ("再起動", ServiceAction::Restart)] {
            if button(ui, label, can_stop) {
                let s = sel.clone().unwrap();
                st.job(Job::ServiceDependents { name: s.name.clone() });
                st.dialog = Some(Dialog::Service { name: s.name, display_name: s.display_name, action, deps: Load::Loading, opened: false });
            }
        }
        if button(ui, "開始の種類…", base) {
            let s = sel.clone().unwrap();
            let choice = s.start_type.filter(|t| StartType::CHOICES.contains(t)).unwrap_or(StartType::Manual);
            st.dialog = Some(Dialog::StartType { name: s.name, display_name: s.display_name, current: s.start_type, choice, opened: false });
        }
        ui.add_space(space::M);
        let no_pid = none.or(sel.as_ref().filter(|s| s.pid == 0).map(|_| "動いていません"));
        if button(ui, "プロセスへ移動", no_pid) {
            let pid = sel.as_ref().map_or(0, |s| s.pid);
            if let Some(r) = st.snap.find_pid(pid) {
                st.procs.scroll_to = Some(r.id);
                st.tab = Tab::Processes;
            }
        }
    });
    ui.add_space(space::XS);
    ui.separator();
    let list = match &st.services.list {
        Load::Ready(v) => v,
        Load::Failed(e) => {
            let text = format!("サービスの一覧を読めませんでした({})", e.message(st.env.elevated));
            widgets::empty_state(ui, &pal, &text, None);
            ui.vertical_centered(|ui| {
                ui.add_space(space::M);
                if ui.button("読み直す").clicked() {
                    st.job_load_services();
                }
            });
            return;
        }
        _ => {
            loading(ui, &pal, "読み込んでいます");
            return;
        }
    };
    let q = st.services.filter.trim().to_lowercase();
    let idx: Vec<usize> = (0..list.len())
        .filter(|&i| {
            let s = &list[i];
            let state_ok = match st.services.state_filter {
                StateFilter::All => true,
                StateFilter::Running => s.state == ServiceState::Running,
                StateFilter::Stopped => s.state == ServiceState::Stopped,
            };
            state_ok && (q.is_empty() || s.name.to_lowercase().contains(&q) || s.display_name.to_lowercase().contains(&q))
        })
        .collect();
    if idx.is_empty() {
        widgets::empty_state(ui, &pal, "一致するサービスはありません", None);
        return;
    }
    let row_h = st.cfg.density.row_h();
    // 最初の幅は窓の幅の割合(狭い窓でも最後の列まで入る)。後は利用者が動かした幅を egui が覚える
    let w = ui.available_width();
    let col = |frac: f32, min: f32| egui_extras::Column::initial((w * frac).max(min)).at_least(min).clip(true);
    let mut click = None;
    let scroll_row = st.services.scroll_to.take().and_then(|n| idx.iter().position(|&i| list[i].name == n));
    let mut tb = TableBuilder::new(ui);
    if let Some(i) = scroll_row {
        tb = tb.scroll_to_row(i, Some(Align::Center));
    }
    tb
        .id_salt("services")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(col(0.12, 60.0))
        .column(col(0.20, 80.0))
        .column(col(0.08, 80.0))
        .column(col(0.10, 60.0))
        .column(col(0.05, 48.0))
        .column(col(0.14, 60.0))
        .column(egui_extras::Column::remainder().at_least(60.0).clip(true))
        .header(row_h + 2.0, |mut h| {
            for (i, t) in ["名前", "表示名", "状態", "開始の種類", "PID", "アカウント", "説明"].into_iter().enumerate() {
                h.col(|ui| {
                    if i == 4 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.strong(t));
                    } else {
                        ui.strong(t);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, idx.len(), |mut row| {
                let s = &list[idx[row.index()]];
                row.set_selected(st.services.selected.as_deref() == Some(s.name.as_str()));
                let pending = st.services.pending.contains_key(&s.name);
                row.col(|ui| {
                    ui.add(egui::Label::new(&s.name).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&s.display_name).truncate());
                });
                row.col(|ui| {
                    if pending {
                        widgets::dot_label(ui, "処理中…", pal.warn);
                    } else {
                        widgets::dot_label(ui, s.state.label(), state_color(&pal, s.state));
                    }
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(s.start_type.map_or(format::DASH, |t| t.label())).truncate());
                });
                row.col(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(if s.pid == 0 { String::new() } else { s.pid.to_string() });
                    });
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&s.account).color(pal.weak)).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&s.description).color(pal.weak)).truncate());
                });
                if row.response().clicked() {
                    click = Some(s.name.clone());
                }
            });
        });
    if click.is_some() {
        st.services.selected = click;
    }
}
