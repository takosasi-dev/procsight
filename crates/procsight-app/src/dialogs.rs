//! 確認と選択の窓 D1〜D9(画面設計書 §9)。開いたときは「やめる」にフォーカスを置き、Enter でも実行しない。Esc は「やめる」。
//! 終了・一時停止・サービスの停止と再起動・開始の種類の変更の Job を積むのは、ここの実行のボタンだけ(INV-5)。
//! 形は共通: 大きな題、本文(画面に収まらない分はスクロール)、対象の一覧はカード、ボタンは右寄せで「やめる」が右端。

use eframe::egui;
use crate::app::{Action, Load, State};
use crate::theme::{self, Palette, size, space};
use crate::widgets;
use egui::{Align, Color32, CornerRadius, Id, Layout, Margin, RichText, Stroke, Ui, Vec2};
use procsight_core::rules::Refusal;
use procsight_win::jobs::{Job, OpError, ServiceAction, StartType, Target};

pub enum Dialog {
    /// D1。終了(複数可)か一時停止
    Confirm { suspend: bool, targets: Vec<Target>, opened: bool },
    /// D2
    Tree { kill: Vec<Target>, excluded: Vec<(Target, Refusal)>, opened: bool },
    /// D3。Ready は (プロセスの割り当て, システムの割り当て)
    Affinity { target: Target, state: Load<(u64, u64)>, mask: u64, opened: bool },
    /// D4
    TreeResult { results: Vec<(Target, Result<(), OpError>)> },
    /// D5。停止と再起動の確認。依存の一覧を取ってから出す
    Service { name: String, display_name: String, action: ServiceAction, deps: Load<Vec<(String, String)>>, opened: bool },
    /// D6
    StartType { name: String, display_name: String, current: Option<StartType>, choice: StartType, opened: bool },
    /// D7
    ExitSuspended { targets: Vec<Target>, opened: bool },
    /// D8
    RestartElevated { opened: bool },
    /// D9。`current` は今の Debugger の別の道具の値
    ReplaceOn { current: Option<String>, opened: bool },
}

/// 窓の幅(画面が狭ければ縮める)
const WIDTH: f32 = 460.0;
/// 一覧の 1 行の高さ。20 行を超えたらスクロール(§9 D1)
const ROW_H: f32 = 20.0;
const MAX_ROWS: f32 = 20.0;
/// 題とボタンの行と縁の分。本文はこれを除いた高さに収める(狭い窓・文字 160% でもボタンを切らない)
const RESERVED_H: f32 = 190.0;
/// 一覧のカードの外に置く物(カードの縁・注意の文)の分
const LIST_SLACK: f32 = 100.0;

/// 窓のボタンの結果
#[derive(PartialEq)]
enum Pick {
    None,
    Run,
    Other,
    Cancel,
}

/// 実行のボタンの見た目
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// 危ない操作(危険の色の地)
    Danger,
    /// 主な操作(強調の色の地)
    Primary,
}

struct Run<'a> {
    label: &'a str,
    enabled: bool,
    kind: Kind,
}

fn run(label: &str, enabled: bool, kind: Kind) -> Option<Run<'_>> {
    Some(Run { label, enabled, kind })
}

fn title(ui: &mut Ui, s: &str) {
    ui.add(egui::Label::new(RichText::new(s).strong().size(size::TITLE)).wrap());
    ui.add_space(space::M);
}

/// 本文。画面の高さに収まらない分はスクロールにして、下のボタンを常に見せる
fn body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let max = (ui.ctx().content_rect().height() - RESERVED_H).max(60.0);
    egui::ScrollArea::vertical().id_salt("dlg_body").max_height(max).auto_shrink([false, true]).show(ui, add).inner
}

/// 対象(名前・表示名)と、その横の弱い文字(PID・サービスの名前)
fn subject(ui: &mut Ui, pal: &Palette, main: &str, sub: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(main).strong());
        ui.label(RichText::new(sub).color(pal.weak));
    });
}

/// 注意の文。色を薄めた地と、左の細い棒
pub(crate) fn note(ui: &mut Ui, color: Color32, text: &str) {
    ui.add_space(space::S);
    let r = egui::Frame::new()
        .fill(widgets::tint(color, 30))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin { left: (space::M + 3.0) as i8, right: space::M as i8, top: (space::S + 2.0) as i8, bottom: (space::S + 2.0) as i8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
        })
        .response;
    let bar = egui::Rect::from_min_size(r.rect.min, Vec2::new(3.0, r.rect.height()));
    ui.painter().rect_filled(bar, CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color);
}

/// 一覧の見出し(太字と件数の札)
fn list_head(ui: &mut Ui, pal: &Palette, text: &str, n: usize) {
    ui.add_space(space::S);
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).strong());
        widgets::pill(ui, format!("{n}"), pal.weak);
    });
}

/// 一覧のカードの高さの上限。20 行までで、画面が低ければ slack(カードの外に置く物の分)を残して切る
fn list_max(ui: &Ui, slack: f32) -> f32 {
    let rows = MAX_ROWS * (ROW_H + space::XS);
    (ui.ctx().content_rect().height() - RESERVED_H - slack).clamp(ROW_H * 3.0, rows)
}

/// カードの中の一覧。max_h を超えたらスクロール(下の注意の文とボタンを見せる)
fn list_card(ui: &mut Ui, pal: &Palette, id: &str, max_h: f32, add: impl FnOnce(&mut Ui)) {
    widgets::card(ui, pal, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt(id).max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = space::XS;
            add(ui)
        });
    });
}

/// 左に名前、右に弱い文字の 1 行
fn name_row(ui: &mut Ui, pal: &Palette, name: &str, right: &str) {
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), ROW_H), Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(right).color(pal.weak).size(size::SMALL));
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(name).truncate());
        });
    });
}

fn target_list(ui: &mut Ui, pal: &Palette, id: &str, slack: f32, list: &[Target]) {
    let max_h = list_max(ui, slack);
    list_card(ui, pal, id, max_h, |ui| {
        for t in list {
            name_row(ui, pal, &t.name, &format!("PID {}", t.id.pid));
        }
    });
}

/// 右寄せの [実行] [別の実行] [やめる]。最初の 1 回は「やめる」にフォーカス(INV-5)。「やめる」は右端
fn buttons(ui: &mut Ui, pal: &Palette, opened: &mut bool, run: Option<Run>, other: Option<&str>, cancel: &str) -> Pick {
    let mut pick = Pick::None;
    let min = Vec2::new(84.0, 28.0);
    ui.add_space(space::L);
    // 高さを決めて置く(with_layout だと残りの高さの真ん中に寄ってしまう)
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), min.y), Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = space::M;
        let c = ui.add(egui::Button::new(cancel).min_size(min));
        if !*opened {
            c.request_focus();
            *opened = true;
        }
        if c.has_focus() {
            // 既定のボタンの印(Enter で押されるのはこれ)
            ui.painter().rect_stroke(c.rect.expand(2.0), CornerRadius::same(5), Stroke::new(2.0, pal.accent), egui::StrokeKind::Outside);
        }
        if c.clicked() {
            pick = Pick::Cancel;
        }
        if let Some(label) = other
            && ui.add(egui::Button::new(label).min_size(min)).clicked()
        {
            pick = Pick::Other;
        }
        if let Some(r) = run {
            let b = match r.kind {
                Kind::Danger => widgets::danger_button(pal, r.label),
                Kind::Primary => widgets::primary_button(pal, r.label),
            };
            if ui.add_enabled(r.enabled, b.min_size(min)).clicked() {
                pick = Pick::Run;
            }
        }
    });
    pick
}

pub fn on_affinity(st: &mut State, target: Target, result: Result<(u64, u64), OpError>) {
    let Some(Dialog::Affinity { target: t, state, mask, .. }) = st.dialog.as_mut() else { return };
    if t.id != target.id {
        return;
    }
    match result {
        Ok((p, s)) => {
            *mask = p;
            *state = Load::Ready((p, s));
        }
        Err(e) => {
            st.dialog = None;
            st.fail(&e);
        }
    }
}

pub fn on_dependents(st: &mut State, name: String, result: Result<Vec<(String, String)>, OpError>) {
    let Some(Dialog::Service { name: n, deps, .. }) = st.dialog.as_mut() else { return };
    if *n != name {
        return;
    }
    match result {
        Ok(v) => *deps = Load::Ready(v),
        Err(e) => {
            st.dialog = None;
            st.fail(&e);
        }
    }
}

fn spinner(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new());
        ui.label(text);
    });
}

pub fn show(st: &mut State, ui: &mut Ui) {
    let Some(mut d) = st.dialog.take() else { return };
    let pal = theme::palette(ui);
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let frame = egui::Frame::popup(ui.style()).inner_margin(Margin::same(space::XL as i8)).corner_radius(CornerRadius::same(8));
    let backdrop = Color32::from_black_alpha(if ui.visuals().dark_mode { 150 } else { 110 });
    let resp = egui::Modal::new(Id::unique("procsight-dialog")).frame(frame).backdrop_color(backdrop).show(&ctx, |ui| {
        // 窓の縁(20×2)と画面の端の隙間を除いた幅
        ui.set_width((screen.width() - space::XL * 2.0 - 24.0).clamp(220.0, WIDTH));
        let mut keep = true;
        match &mut d {
            Dialog::Confirm { suspend, targets, opened } => {
                title(ui, if *suspend { "プロセスを一時停止しますか" } else { "プロセスを終了しますか" });
                body(ui, |ui| {
                    target_list(ui, &pal, "targets", LIST_SLACK, targets);
                    if !*suspend {
                        note(ui, pal.warn, "保存していない作業は失われます");
                    }
                });
                let label = if *suspend { "一時停止する" } else { "終了する" };
                match buttons(ui, &pal, opened, run(label, true, Kind::Danger), None, "やめる") {
                    Pick::Run => {
                        if *suspend {
                            if let Some(t) = targets.first() {
                                st.job(Job::Suspend { target: t.clone() });
                            }
                        } else {
                            st.job(Job::Terminate { targets: targets.clone(), tree: false });
                        }
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::Tree { kill, excluded, opened } => {
                title(ui, "ツリーを終了しますか");
                body(ui, |ui| {
                    // 件数と注意の文を先に(一覧が長くても見える)
                    ui.label(format!("{} 個を終了します。{} 個は除外します", kill.len(), excluded.len()));
                    note(ui, pal.warn, "保存していない作業は失われます");
                    // 除外の一覧は 1 件 2 行。画面が低いときは 2 件分まで
                    let ex_h = list_max(ui, LIST_SLACK + 160.0).min(excluded.len() as f32 * (ROW_H * 2.0 + space::S) + space::XS);
                    let kill_slack = LIST_SLACK + 40.0 + if excluded.is_empty() { 0.0 } else { ex_h + 60.0 };
                    list_head(ui, &pal, "終了する物", kill.len());
                    target_list(ui, &pal, "kill", kill_slack, kill);
                    if !excluded.is_empty() {
                        list_head(ui, &pal, "除外する物", excluded.len());
                        list_card(ui, &pal, "excluded", ex_h, |ui| {
                            for (t, r) in excluded.iter() {
                                name_row(ui, &pal, &t.name, &format!("PID {}", t.id.pid));
                                ui.add(egui::Label::new(RichText::new(r.message()).color(pal.weak).size(size::SMALL)).wrap());
                                ui.add_space(space::XS);
                            }
                        });
                    }
                });
                let label = format!("{} 個を終了する", kill.len());
                match buttons(ui, &pal, opened, run(&label, !kill.is_empty(), Kind::Danger), None, "やめる") {
                    Pick::Run => {
                        st.job(Job::Terminate { targets: kill.clone(), tree: true });
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::Affinity { target, state, mask, opened } => {
                title(ui, "CPU の割り当て");
                let mut ready = false;
                body(ui, |ui| {
                    subject(ui, &pal, &target.name, &format!("PID {}", target.id.pid));
                    ui.add_space(space::S);
                    match state {
                        Load::Ready((_, sys)) => {
                            ready = true;
                            let sys = *sys;
                            widgets::card(ui, &pal, |ui| {
                                ui.set_width(ui.available_width());
                                // 8・4・2 の段で、幅に入る一番多い列(CPU の数はふつう 2 のべき)
                                let fit = (ui.available_width() / 84.0).floor() as usize;
                                let cols = [8, 4, 2].into_iter().find(|&c| c <= fit).unwrap_or(2);
                                egui::ScrollArea::vertical().id_salt("aff").max_height(MAX_ROWS * ROW_H).auto_shrink([false, true]).show(ui, |ui| {
                                    egui::Grid::new("aff_grid").num_columns(cols).min_col_width(70.0).show(ui, |ui| {
                                        let mut col = 0;
                                        for i in 0..64u32 {
                                            if sys & (1u64 << i) == 0 {
                                                continue;
                                            }
                                            let mut on = *mask & (1u64 << i) != 0;
                                            if ui.checkbox(&mut on, format!("CPU {i}")).changed() {
                                                if on { *mask |= 1u64 << i } else { *mask &= !(1u64 << i) }
                                            }
                                            col += 1;
                                            if col % cols == 0 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                                });
                            });
                            ui.add_space(space::S);
                            ui.horizontal(|ui| {
                                if ui.button("すべて選ぶ").clicked() {
                                    *mask = sys;
                                }
                                ui.label(RichText::new(format!("{} 個を選んでいます", mask.count_ones())).color(pal.weak));
                            });
                            if *mask == 0 {
                                note(ui, pal.danger, "1 つ以上選んでください");
                            }
                        }
                        _ => spinner(ui, "取得しています"),
                    }
                });
                match buttons(ui, &pal, opened, run("確定", ready && *mask != 0, Kind::Primary), None, "やめる") {
                    Pick::Run => {
                        st.job(Job::SetAffinity { target: target.clone(), mask: *mask });
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::TreeResult { results } => {
                title(ui, "ツリーの終了の結果");
                let elevated = st.env.elevated;
                body(ui, |ui| {
                    let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
                    ui.horizontal(|ui| {
                        widgets::pill(ui, format!("終了 {ok}"), pal.ok);
                        if ok < results.len() {
                            widgets::pill(ui, format!("失敗 {}", results.len() - ok), pal.danger);
                        }
                    });
                    ui.add_space(space::S);
                    let max_h = list_max(ui, LIST_SLACK);
                    list_card(ui, &pal, "results", max_h, |ui| {
                        for (t, r) in results.iter() {
                            let (color, right) = match r {
                                Ok(()) => (pal.ok, "終了しました"),
                                Err(_) => (pal.danger, "失敗"),
                            };
                            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), ROW_H), Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(right).color(color).size(size::SMALL));
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                    ui.painter().circle_filled(rect.center(), 3.5, color);
                                    ui.add(egui::Label::new(format!("{}(PID {})", t.name, t.id.pid)).truncate());
                                });
                            });
                            if let Err(e) = r {
                                ui.add(egui::Label::new(RichText::new(e.message(elevated)).color(pal.danger).size(size::SMALL)).wrap());
                                ui.add_space(space::XS);
                            }
                        }
                    });
                });
                let mut o = true;
                if buttons(ui, &pal, &mut o, None, None, "閉じる") == Pick::Cancel {
                    keep = false;
                }
            }
            Dialog::Service { name, display_name, action, deps, opened } => {
                let restart = *action == ServiceAction::Restart;
                title(ui, if restart { "サービスを再起動しますか" } else { "サービスを停止しますか" });
                let mut ready = false;
                body(ui, |ui| {
                    subject(ui, &pal, display_name, &format!("({name})"));
                    match deps {
                        Load::Ready(v) => {
                            ready = true;
                            if !v.is_empty() {
                                note(ui, pal.warn, "次のサービスも停止します");
                                ui.add_space(space::S);
                                let max_h = list_max(ui, LIST_SLACK);
                                list_card(ui, &pal, "deps", max_h, |ui| {
                                    for (n, dn) in v.iter() {
                                        name_row(ui, &pal, dn, n);
                                    }
                                });
                            }
                        }
                        _ => {
                            ui.add_space(space::S);
                            spinner(ui, "依存するサービスを調べています");
                        }
                    }
                });
                let label = if restart { "再起動する" } else { "停止する" };
                match buttons(ui, &pal, opened, run(label, ready, Kind::Danger), None, "やめる") {
                    Pick::Run => {
                        crate::services_view::mark_pending(st, name, display_name);
                        // 窓に出した依存だけを止めてよい(INV-5)
                        let confirmed = match deps {
                            Load::Ready(v) => v.iter().map(|(n, _)| n.clone()).collect(),
                            _ => Vec::new(),
                        };
                        st.job(Job::ServiceControl {
                            name: name.clone(),
                            display_name: display_name.clone(),
                            action: *action,
                            confirmed,
                        });
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::StartType { name, display_name, current, choice, opened } => {
                title(ui, "開始の種類を変えますか");
                body(ui, |ui| {
                    subject(ui, &pal, display_name, &format!("({name})"));
                    ui.add_space(space::S);
                    widgets::card(ui, &pal, |ui| {
                        ui.set_width(ui.available_width());
                        egui::Grid::new("start_type_grid").num_columns(2).spacing([space::L, space::M]).show(ui, |ui| {
                            ui.label(RichText::new("今の種類").color(pal.weak));
                            ui.label(current.map_or("—", |c| c.label()));
                            ui.end_row();
                            let l = ui.label(RichText::new("新しい種類").color(pal.weak));
                            egui::ComboBox::from_id_salt("start_type")
                                .selected_text(choice.label())
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for c in StartType::CHOICES {
                                        ui.selectable_value(choice, c, c.label());
                                    }
                                })
                                .response
                                .labelled_by(l.id);
                            ui.end_row();
                        });
                    });
                    note(ui, pal.warn, "Windows の起動や動作に要るサービスを無効にすると、起動できなくなることがあります");
                });
                match buttons(ui, &pal, opened, run("変更する", Some(*choice) != *current, Kind::Danger), None, "やめる") {
                    Pick::Run => {
                        crate::services_view::mark_pending(st, name, display_name);
                        st.job(Job::SetStartType { name: name.clone(), display_name: display_name.clone(), start_type: *choice });
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::ExitSuspended { targets, opened } => {
                title(ui, "一時停止中のプロセスがあります");
                body(ui, |ui| target_list(ui, &pal, "suspended", LIST_SLACK, targets));
                match buttons(ui, &pal, opened, run("再開してから終了", true, Kind::Primary), Some("そのまま終了"), "やめる") {
                    Pick::Run => {
                        st.exit_after_resume = true;
                        st.job(Job::ResumeMany { targets: targets.clone() });
                        keep = false;
                    }
                    Pick::Other => {
                        st.actions.push(Action::Exit);
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    Pick::None => {}
                }
            }
            Dialog::RestartElevated { opened } => {
                title(ui, "管理者で再起動");
                body(ui, |ui| {
                    ui.add(egui::Label::new("ProcSight を管理者で起動し直します。この窓は閉じます。").wrap());
                });
                match buttons(ui, &pal, opened, run("再起動する", true, Kind::Primary), None, "やめる") {
                    Pick::Run => {
                        st.job(Job::RestartElevated);
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::ReplaceOn { current, opened } => {
                title(ui, "Ctrl+Shift+Esc で ProcSight を開くようにしますか");
                body(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(crate::settings::REPLACE_TEXT).color(pal.weak)).wrap());
                    if let Some(v) = current {
                        note(ui, pal.warn, &format!("今の登録: {v}。この値を覚えておき、オフにしたときに戻します"));
                    }
                });
                match buttons(ui, &pal, opened, run("オンにする", true, Kind::Danger), None, "やめる") {
                    Pick::Run => {
                        st.job(Job::ReplaceEnable);
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
        }
        keep
    });
    if resp.inner && !resp.should_close() {
        // 窓の中のボタンで別の窓を開いた(例: なし)ときは上書きしない
        if st.dialog.is_none() {
            st.dialog = Some(d);
        }
    }
}
