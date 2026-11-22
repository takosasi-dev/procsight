//! 確認と選択の窓 D1〜D9(画面設計書 §9)。開いたときは「やめる」にフォーカスを置き、Enter でも実行しない。Esc は「やめる」。
//! 終了・一時停止・サービスの停止と再起動・開始の種類の変更の Job を積むのは、ここの実行のボタンだけ(INV-5)。
//! 確認の窓には対象の名前と PID(サービスは表示名と名前)を出す。

use eframe::egui;
use crate::app::{Action, Load, State};
use crate::theme;
use egui::{Id, RichText, Ui};
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

/// 窓のボタンの結果
#[derive(PartialEq)]
enum Pick {
    None,
    Run,
    Other,
    Cancel,
}

fn target_list(ui: &mut Ui, list: &[Target]) {
    egui::ScrollArea::vertical().max_height(20.0 * 18.0).auto_shrink([false, true]).show(ui, |ui| {
        for t in list {
            ui.label(format!("{}(PID {})", t.name, t.id.pid));
        }
    });
}

/// [実行] [別の実行] [やめる] を並べる。最初の 1 回は「やめる」にフォーカス
fn buttons(ui: &mut Ui, opened: &mut bool, run: Option<(&str, bool)>, other: Option<&str>, cancel: &str) -> Pick {
    let mut pick = Pick::None;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if let Some((label, enabled)) = run
            && ui.add_enabled(enabled, egui::Button::new(label)).clicked()
        {
            pick = Pick::Run;
        }
        if let Some(label) = other
            && ui.button(label).clicked()
        {
            pick = Pick::Other;
        }
        let c = ui.button(cancel);
        if !*opened {
            c.request_focus();
            *opened = true;
        }
        if c.clicked() {
            pick = Pick::Cancel;
        }
    });
    pick
}

fn title(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).strong().size(16.0));
    ui.add_space(6.0);
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

pub fn show(st: &mut State, ui: &mut Ui) {
    let Some(mut d) = st.dialog.take() else { return };
    let pal = theme::palette(ui);
    let ctx = ui.ctx().clone();
    let resp = egui::Modal::new(Id::unique("procsight-dialog")).show(&ctx, |ui| {
        ui.set_width(460.0);
        let mut keep = true;
        match &mut d {
            Dialog::Confirm { suspend, targets, opened } => {
                title(ui, if *suspend { "プロセスを一時停止しますか" } else { "プロセスを終了しますか" });
                target_list(ui, targets);
                if !*suspend {
                    ui.add_space(4.0);
                    ui.label(RichText::new("保存していない作業は失われます").color(pal.warn));
                }
                let run = if *suspend { "一時停止する" } else { "終了する" };
                match buttons(ui, opened, Some((run, true)), None, "やめる") {
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
                ui.label(format!("{} 個を終了します。{} 個は除外します", kill.len(), excluded.len()));
                ui.add_space(4.0);
                ui.label(RichText::new("終了する物").strong());
                target_list(ui, kill);
                if !excluded.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new("除外する物").strong());
                    egui::ScrollArea::vertical().id_salt("excluded").max_height(8.0 * 18.0).show(ui, |ui| {
                        for (t, r) in excluded.iter() {
                            ui.label(format!("{}(PID {}): {}", t.name, t.id.pid, r.message()));
                        }
                    });
                }
                ui.label(RichText::new("保存していない作業は失われます").color(pal.warn));
                let run = format!("{} 個を終了する", kill.len());
                match buttons(ui, opened, Some((&run, !kill.is_empty())), None, "やめる") {
                    Pick::Run => {
                        st.job(Job::Terminate { targets: kill.clone(), tree: true });
                        keep = false;
                    }
                    Pick::Cancel => keep = false,
                    _ => {}
                }
            }
            Dialog::Affinity { target, state, mask, opened } => {
                title(ui, &format!("CPU の割り当て: {}(PID {})", target.name, target.id.pid));
                let mut ready = false;
                match state {
                    Load::Ready((_, sys)) => {
                        ready = true;
                        let sys = *sys;
                        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                            egui::Grid::new("aff").num_columns(4).show(ui, |ui| {
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
                                    if col % 4 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                        });
                        if ui.button("すべて選ぶ").clicked() {
                            *mask = sys;
                        }
                        if *mask == 0 {
                            ui.label(RichText::new("1 つ以上選んでください").color(pal.danger));
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label("取得しています");
                        });
                    }
                }
                match buttons(ui, opened, Some(("確定", ready && *mask != 0)), None, "やめる") {
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
                egui::ScrollArea::vertical().max_height(20.0 * 18.0).show(ui, |ui| {
                    for (t, r) in results.iter() {
                        let (text, color) = match r {
                            Ok(()) => (format!("{}(PID {}): 終了しました", t.name, t.id.pid), ui.visuals().text_color()),
                            Err(e) => (format!("{}(PID {}): {}", t.name, t.id.pid, e.message(st.env.elevated)), pal.danger),
                        };
                        ui.label(RichText::new(text).color(color));
                    }
                });
                let mut o = true;
                if buttons(ui, &mut o, None, None, "閉じる") == Pick::Cancel {
                    keep = false;
                }
            }
            Dialog::Service { name, display_name, action, deps, opened } => {
                let restart = *action == ServiceAction::Restart;
                title(ui, if restart { "サービスを再起動しますか" } else { "サービスを停止しますか" });
                ui.label(format!("{display_name}({name})"));
                let mut ready = false;
                match deps {
                    Load::Ready(v) => {
                        ready = true;
                        if !v.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new("次のサービスも停止します").color(pal.warn));
                            egui::ScrollArea::vertical().max_height(10.0 * 18.0).show(ui, |ui| {
                                for (n, dn) in v.iter() {
                                    ui.label(format!("{dn}({n})"));
                                }
                            });
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label("依存するサービスを調べています");
                        });
                    }
                }
                let run = if restart { "再起動する" } else { "停止する" };
                match buttons(ui, opened, Some((run, ready)), None, "やめる") {
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
                ui.label(format!("{display_name}({name})"));
                ui.label(format!("今の種類: {}", current.map_or("—", |c| c.label())));
                ui.horizontal(|ui| {
                    let l = ui.label("新しい種類");
                    egui::ComboBox::from_id_salt("start_type")
                        .selected_text(choice.label())
                        .show_ui(ui, |ui| {
                            for c in StartType::CHOICES {
                                ui.selectable_value(choice, c, c.label());
                            }
                        })
                        .response
                        .labelled_by(l.id);
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Windows の起動や動作に要るサービスを無効にすると、起動できなくなることがあります").color(pal.warn),
                );
                match buttons(ui, opened, Some(("変更する", Some(*choice) != *current)), None, "やめる") {
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
                target_list(ui, targets);
                match buttons(ui, opened, Some(("再開してから終了", true)), Some("そのまま終了"), "やめる") {
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
                ui.label("ProcSight を管理者で起動し直します。この窓は閉じます。");
                match buttons(ui, opened, Some(("再起動する", true)), None, "やめる") {
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
                ui.label(crate::settings::REPLACE_TEXT);
                if let Some(v) = current {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("今の登録: {v}。この値を覚えておき、オフにしたときに戻します")).color(pal.warn));
                }
                match buttons(ui, opened, Some(("オンにする", true)), None, "やめる") {
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
