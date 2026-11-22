//! S8 ファイルを掴んでいるプロセス(画面設計書 §9.1・FR-35・D-25)。ドロップと「ファイルを選ぶ…」(Job::PickFiles)で受け、
//! Job::FindLocking で探す。フォルダと 101 個以上は断る(E-26)。触れないプロセスが動いている間は「探す」を灰色にする。
//! 結果を選ぶと S1 のその行へ移る(触れないプロセスなら選ぶだけで詳細は出さない)。

use eframe::egui;
use crate::app::{Load, State, Tab};
use crate::theme::{self, space};
use crate::widgets;
use egui::{Align, Align2, Layout, RichText, Sense, Ui, Vec2};
use egui_extras::TableBuilder;
use procsight_core::ProcId;
use procsight_win::jobs::{Job, LockingProcess, OpError};

pub const MAX_FILES: usize = 100;

#[derive(Default)]
pub struct LocksView {
    pub open: bool,
    pub files: Vec<String>,
    /// 受け付けなかった理由(E-26)
    pub problem: Option<&'static str>,
    pub result: Load<Vec<LockingProcess>>,
    pub selected: Option<usize>,
}

pub fn on_picked(st: &mut State, paths: Vec<String>) {
    if !paths.is_empty() {
        set_files(st, paths);
    }
}

pub fn on_result(st: &mut State, paths: Vec<String>, r: Result<Vec<LockingProcess>, OpError>) {
    if paths != st.locks.files {
        return;
    }
    st.locks.selected = None;
    st.locks.result = match r {
        Ok(v) => Load::Ready(v),
        Err(e) => Load::Failed(e),
    };
}

/// 受けたファイルを確かめる。フォルダは調べない(事実12)
pub fn check_files(paths: &[String]) -> Result<(), &'static str> {
    if paths.len() > MAX_FILES {
        return Err("一度に 100 ファイルまでです");
    }
    if paths.iter().any(|p| std::path::Path::new(p).is_dir()) {
        return Err("フォルダは調べられません。中のファイルを選んでください");
    }
    Ok(())
}

fn set_files(st: &mut State, paths: Vec<String>) {
    st.locks.result = Load::NotAsked;
    st.locks.selected = None;
    match check_files(&paths) {
        Ok(()) => {
            st.locks.problem = None;
            st.locks.files = paths;
        }
        Err(why) => {
            st.locks.problem = Some(why);
            st.locks.files.clear();
        }
    }
}

fn file_name(p: &str) -> &str {
    procsight_core::startup::file_name(p)
}

fn go_to(st: &mut State, l: &LockingProcess) {
    let id = ProcId { pid: l.pid, create_time: l.create_time };
    let id = if st.snap.find(id).is_some() { Some(id) } else { st.snap.find_pid(l.pid).map(|r| r.id) };
    if let Some(id) = id {
        st.procs.scroll_to = Some(id);
        st.tab = Tab::Processes;
    }
}

pub fn window(st: &mut State, ui: &mut Ui) {
    if !st.locks.open {
        return;
    }
    let dropped: Vec<String> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_string_lossy().to_string()).filter(|p| !p.is_empty()).collect());
    if !dropped.is_empty() {
        set_files(st, dropped);
    }
    let pal = theme::palette(ui);
    let mut open = true;
    let ctx = ui.ctx().clone();
    let touch_running = !st.snap.touch_free.is_empty();
    // 狭い窓・大きな文字でもはみ出さない大きさ(題の帯の分を引く)
    let screen = ctx.content_rect();
    let max = Vec2::new(screen.width() - 24.0, screen.height() - 64.0);
    egui::Window::new("ファイルを掴んでいるプロセスを探す")
        .open(&mut open)
        .collapsible(false)
        .default_size(Vec2::new(640.0, 440.0).min(max))
        .min_size(Vec2::new(600.0, 380.0).min(max))
        .max_size(max)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .show(&ctx, |ui| {
        if touch_running {
            crate::dialogs::note(ui, pal.warn, "触れないプロセスが動いている間は、この検索を使えません");
            ui.add_space(space::S);
        }
        // ファイルを受ける場所
        widgets::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label("ここにファイルをドロップするか、");
                if ui.button("ファイルを選ぶ…").clicked() {
                    st.job_picker(Job::PickFiles);
                }
            });
            if let Some(p) = st.locks.problem {
                ui.add(egui::Label::new(RichText::new(p).color(pal.danger)).wrap());
            }
            if !st.locks.files.is_empty() {
                ui.add_space(space::XS);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("(選んだファイル: {} 個)", st.locks.files.len())).color(pal.weak));
                    for p in st.locks.files.iter().take(6) {
                        widgets::pill(ui, file_name(p), pal.accent);
                    }
                    if st.locks.files.len() > 6 {
                        ui.label(RichText::new("…").color(pal.weak));
                    }
                });
            }
        });
        ui.add_space(space::S);
        let mut go = None;
        // 結果の表の場所(ボタンの行を下に置く)
        let h = (ui.available_height() - 40.0).max(60.0);
        let w = ui.available_width();
        ui.allocate_ui(egui::vec2(w, h), |ui| {
            ui.set_min_size(egui::vec2(w, h));
            result_area(st, ui, &pal, &mut go)
        });
        ui.add_space(space::S);
        let mut search = false;
        ui.horizontal(|ui| {
            let can_go = st.locks.selected.is_some() && matches!(st.locks.result, Load::Ready(_));
            if ui.add_enabled(can_go, egui::Button::new("プロセスの行へ移動")).clicked()
                && let (Some(i), Load::Ready(list)) = (st.locks.selected, &st.locks.result)
                && let Some(l) = list.get(i).cloned()
            {
                go_to(st, &l);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let why = if touch_running {
                    Some("触れないプロセスが動いている間は、この検索を使えません")
                } else if st.locks.files.is_empty() {
                    Some("ファイルを選んでください")
                } else {
                    None
                };
                let r = ui.add_enabled(why.is_none() && !matches!(st.locks.result, Load::Loading), widgets::primary_button(&pal, "探す").min_size(Vec2::new(84.0, 0.0)));
                if let Some(w) = why {
                    r.on_disabled_hover_text(w);
                } else if r.clicked() {
                    search = true;
                }
            });
        });
        if let Some((i, double)) = go {
            st.locks.selected = Some(i);
            if double && let Load::Ready(list) = &st.locks.result && let Some(l) = list.get(i).cloned() {
                go_to(st, &l);
            }
        }
        if search {
            st.locks.result = Load::Loading;
            st.job(Job::FindLocking { paths: st.locks.files.clone() });
        }
    });
    if !open {
        st.locks.open = false;
    }
}

/// 結果の表と、探している・見つからない・失敗の文
fn result_area(st: &State, ui: &mut Ui, pal: &crate::theme::Palette, go: &mut Option<(usize, bool)>) {
    match &st.locks.result {
            Load::NotAsked => {
                widgets::empty_state(ui, pal, "ファイルを選んで「探す」を押してください", None);
            }
            Load::Loading => crate::services_view::loading(ui, pal, "探しています"),
            Load::Failed(e) => {
                widgets::empty_state(ui, pal, &e.message(st.env.elevated), None);
            }
            Load::Ready(list) if list.is_empty() => {
                widgets::empty_state(ui, pal, "このファイルを使っているプロセスは見つかりませんでした", None);
            }
            Load::Ready(list) => {
                let file_col = if st.locks.files.len() == 1 {
                    file_name(&st.locks.files[0]).to_string()
                } else {
                    format!("選んだ {} 個のどれか", st.locks.files.len())
                };
                let row_h = st.cfg.density.row_h();
                let w = ui.available_width();
                TableBuilder::new(ui)
                    .id_salt("locks_table")
                    .striped(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(egui_extras::Column::initial((w * 0.34).max(80.0)).at_least(80.0).clip(true))
                    .column(egui_extras::Column::initial((w * 0.1).max(50.0)).at_least(50.0))
                    .column(egui_extras::Column::initial((w * 0.18).max(60.0)).at_least(50.0).clip(true))
                    .column(egui_extras::Column::remainder().at_least(60.0).clip(true))
                    .header(row_h + 2.0, |mut hd| {
                        for t in ["名前", "PID", "種類", "ファイル"] {
                            hd.col(|ui| {
                                ui.strong(t);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_h, list.len(), |mut row| {
                            let i = row.index();
                            let l = &list[i];
                            row.set_selected(st.locks.selected == Some(i));
                            row.col(|ui| {
                                let name = match &l.service_short_name {
                                    Some(s) => format!("{}(サービス {s})", l.app_name),
                                    None => l.app_name.clone(),
                                };
                                ui.add(egui::Label::new(name).truncate());
                            });
                            row.col(|ui| {
                                ui.label(l.pid.to_string());
                            });
                            row.col(|ui| {
                                ui.label(&l.kind);
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(&file_col).truncate());
                            });
                            let r = row.response();
                            if r.double_clicked() {
                                *go = Some((i, true));
                            } else if r.clicked() {
                                *go = Some((i, false));
                            }
                        });
                    });
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_folders_and_too_many() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        assert_eq!(check_files(&[dir]), Err("フォルダは調べられません。中のファイルを選んでください"));
        let many: Vec<String> = (0..101).map(|i| format!("C:\\nope\\{i}.txt")).collect();
        assert_eq!(check_files(&many), Err("一度に 100 ファイルまでです"));
        assert_eq!(check_files(&["C:\\nope\\a.txt".into()]), Ok(()));
    }
}
