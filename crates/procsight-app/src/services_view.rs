//! S4 サービス(画面設計書 §6.2・FR-41〜FR-44)。絞り込み・状態の範囲・開始・停止・再起動・開始の種類・プロセスへ移動。
//! 停止と再起動は依存の一覧を取ってから D5、開始の種類は D6 を通す(INV-5)。昇格していなければ操作を灰色にする。
//! 操作の間は行に「処理中…」を出す(30 秒の打ち切りは Worker。E-31)。

use eframe::egui;
use crate::app::{Load, State, Tab};
use crate::dialogs::Dialog;
use crate::theme;
use egui::{Align, Key, Layout, RichText, Sense, Ui};
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

fn button(ui: &mut Ui, label: &str, why: Option<&str>) -> bool {
    let r = ui.add_enabled(why.is_none(), egui::Button::new(label));
    match why {
        Some(w) => r.on_disabled_hover_text(w).clicked(),
        None => r.clicked(),
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
    ui.horizontal(|ui| {
        let resp = ui.add(egui::TextEdit::singleline(&mut st.services.filter).hint_text("名前・表示名で絞り込み(Ctrl+F)").desired_width(220.0));
        if std::mem::take(&mut st.services.focus_filter) {
            resp.request_focus();
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            st.services.filter.clear();
        }
        let f = &mut st.services.state_filter;
        ui.selectable_value(f, StateFilter::All, "すべて");
        ui.selectable_value(f, StateFilter::Running, "実行中");
        ui.selectable_value(f, StateFilter::Stopped, "停止");
        ui.add_space(12.0);
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
        let no_pid = none.or(sel.as_ref().filter(|s| s.pid == 0).map(|_| "動いていません"));
        if button(ui, "プロセスへ移動", no_pid) {
            let pid = sel.as_ref().map_or(0, |s| s.pid);
            if let Some(r) = st.snap.find_pid(pid) {
                st.procs.scroll_to = Some(r.id);
                st.tab = Tab::Processes;
            }
        }
    });
    ui.separator();
    let list = match &st.services.list {
        Load::Ready(v) => v,
        Load::Failed(e) => {
            ui.label(RichText::new(format!("サービスの一覧を読めませんでした({})", e.message(st.env.elevated))).color(pal.danger));
            return;
        }
        _ => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label("読み込んでいます");
            });
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
        ui.label("一致するサービスはありません");
        return;
    }
    let mut click = None;
    TableBuilder::new(ui)
        .id_salt("services")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(egui_extras::Column::initial(140.0).clip(true))
        .column(egui_extras::Column::initial(230.0).clip(true))
        .column(egui_extras::Column::initial(80.0))
        .column(egui_extras::Column::initial(120.0))
        .column(egui_extras::Column::initial(60.0))
        .column(egui_extras::Column::initial(150.0).clip(true))
        .column(egui_extras::Column::remainder().clip(true))
        .header(22.0, |mut h| {
            for t in ["名前", "表示名", "状態", "開始の種類", "PID", "アカウント", "説明"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(20.0, idx.len(), |mut row| {
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
                        ui.label(RichText::new("処理中…").color(pal.warn));
                    } else {
                        let warn = !matches!(s.state, ServiceState::Running | ServiceState::Stopped | ServiceState::Paused);
                        if warn {
                            ui.label(RichText::new(s.state.label()).color(pal.warn));
                        } else {
                            ui.label(s.state.label());
                        }
                    }
                });
                row.col(|ui| {
                    ui.label(s.start_type.map_or(format::DASH, |t| t.label()));
                });
                row.col(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(if s.pid == 0 { String::new() } else { s.pid.to_string() });
                    });
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&s.account).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&s.description).truncate());
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
