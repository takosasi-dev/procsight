//! S3 スタートアップ(画面設計書 §6.1・FR-38〜FR-40)。一覧の表と、有効・無効・場所を開く・取り直す。
//! 全ユーザーの項目は昇格していなければ灰色(FR-39)。状態が「不明」の項目も切り替えない(E-30)。

use eframe::egui;
use crate::app::{Load, State};
use crate::theme;
use egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::TableBuilder;
use procsight_core::format;
use procsight_core::startup::{Approved, StartupItem, StartupLocation};
use procsight_win::jobs::{Job, OpError};

#[derive(Default)]
pub struct StartupView {
    pub items: Load<Vec<StartupItem>>,
    pub selected: Option<(String, StartupLocation)>,
}

pub fn on_loaded(st: &mut State, r: Result<Vec<StartupItem>, OpError>) {
    st.startup.items = match r {
        Ok(v) => Load::Ready(v),
        Err(e) => Load::Failed(e),
    };
}

pub fn on_set(st: &mut State, name: String, enabled: bool, r: Result<(), OpError>) {
    match r {
        Ok(()) => {
            st.ok(format!("『{name}』を{}しました", if enabled { "有効に" } else { "無効に" }));
            crate::log::info("startup_set", &format!("enabled={enabled} result=ok"));
            st.job_load_startup();
        }
        Err(e) => st.fail(&e),
    }
}

/// 切り替えの灰色の理由
fn refusal(st: &State, it: &StartupItem) -> Option<&'static str> {
    if it.location.needs_admin() && !st.env.elevated {
        Some("管理者で再起動すると変えられます")
    } else if it.approved == Approved::Unknown {
        Some("記録の形が想定と違うため、切り替えません")
    } else {
        None
    }
}

fn button(ui: &mut Ui, label: &str, why: Option<&str>) -> bool {
    let r = ui.add_enabled(why.is_none(), egui::Button::new(label));
    match why {
        Some(w) => r.on_disabled_hover_text(w).clicked(),
        None => r.clicked(),
    }
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    if st.startup.items.is_not_asked() {
        st.job_load_startup();
    }
    let pal = theme::palette(ui);
    let sel = match (&st.startup.items, &st.startup.selected) {
        (Load::Ready(v), Some((n, l))) => v.iter().find(|i| &i.name == n && i.location == *l).cloned(),
        _ => None,
    };
    ui.horizontal(|ui| {
        let none = sel.is_none().then_some("項目を選んでください");
        let why = none.or_else(|| sel.as_ref().and_then(|i| refusal(st, i)));
        if button(ui, "有効にする", why.or(sel.as_ref().filter(|i| i.approved == Approved::Enabled).map(|_| "すでに有効です"))) {
            let it = sel.clone().unwrap();
            st.job(Job::SetStartup { name: it.name, location: it.location, enabled: true });
        }
        let already_off = sel.as_ref().filter(|i| matches!(i.approved, Approved::Disabled(_))).map(|_| "すでに無効です");
        if button(ui, "無効にする", why.or(already_off)) {
            let it = sel.clone().unwrap();
            st.job(Job::SetStartup { name: it.name, location: it.location, enabled: false });
        }
        let no_exe = none.or(sel.as_ref().filter(|i| i.exe_path.is_none() || !i.exe_exists).map(|_| "ファイルが見つかりません"));
        if button(ui, "ファイルの場所を開く", no_exe) {
            st.job(Job::OpenLocation { path: sel.as_ref().and_then(|i| i.exe_path.clone()).unwrap_or_default() });
        }
        ui.add_space(12.0);
        if ui.button("取り直す").on_hover_text("F5").clicked() {
            st.job_load_startup();
        }
    });
    ui.separator();
    let items = match &st.startup.items {
        Load::Ready(v) => v.clone(),
        Load::Failed(e) => {
            ui.label(RichText::new(format!("スタートアップの一覧を読めませんでした({})", e.message(st.env.elevated))).color(pal.danger));
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
    if items.is_empty() {
        ui.label("ログオン時に動く項目はありません");
        return;
    }
    let mut click = None;
    TableBuilder::new(ui)
        .id_salt("startup")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(egui_extras::Column::initial(170.0).clip(true))
        .column(egui_extras::Column::initial(170.0).clip(true))
        .column(egui_extras::Column::initial(120.0))
        .column(egui_extras::Column::initial(230.0).clip(true))
        .column(egui_extras::Column::initial(150.0))
        .column(egui_extras::Column::remainder().clip(true))
        .header(22.0, |mut h| {
            for t in ["名前", "発行元", "状態", "場所", "無効にした日時", "コマンド"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(20.0, items.len(), |mut row| {
                let it = &items[row.index()];
                row.set_selected(st.startup.selected.as_ref().is_some_and(|(n, l)| *n == it.name && *l == it.location));
                row.col(|ui| {
                    ui.add(egui::Label::new(&it.name).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(it.publisher.as_deref().unwrap_or(format::DASH)).truncate());
                });
                row.col(|ui| {
                    ui.label(it.approved.label());
                    if !it.exe_exists {
                        ui.label(RichText::new("ファイルなし").color(pal.danger));
                    }
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(it.location.label()).truncate());
                });
                row.col(|ui| {
                    let when = match it.approved {
                        Approved::Disabled(ft) if ft != 0 => format::filetime(ft, st.env.utc_offset_minutes)[..16].to_string(),
                        _ => format::DASH.into(),
                    };
                    ui.label(when);
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&it.command).truncate());
                });
                if row.response().clicked() {
                    click = Some((it.name.clone(), it.location));
                }
            });
        });
    if click.is_some() {
        st.startup.selected = click;
    }
}
