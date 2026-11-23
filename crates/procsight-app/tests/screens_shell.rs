//! 窓の共通の部分(上の列・帯・状態の行)・コマンド パレット・字形(統合と担当 D)を、ダーク・ライトで PNG に描いて確かめる。
#![allow(unused_imports)]

mod common;

use common::*;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use procsight_app::app::{Action, BandButton, BandKey, BandKind, Load, State, Tab};
use procsight_app::details::DTab;
use procsight_app::dialogs::Dialog;
use procsight_app::perf::Page;
use procsight_app::settings::SPage;
use procsight_app::{demo, fonts};
use procsight_core::config::Theme;
use procsight_core::rules::Refusal;
use procsight_win::jobs::{Job, OpError, ServiceAction, StartType, Target};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

#[test]
fn s1_bands_and_elevated() {
    shoot("s1-bands", BIG, || {
        let mut st = demo::state();
        st.env.elevated = true;
        st.set_paused(true);
        st.fail(&OpError::Win32(5));
        st.ok("notepad.exe(PID 7000)を終了しました");
        st
    });
    shoot("s1-bands-buttons", BIG, || {
        let mut st = demo::state();
        st.fail(&OpError::Win32(5));
        st.push_band(procsight_app::app::Band {
            kind: BandKind::Warn,
            key: BandKey::SuspendedLeft,
            text: "前回一時停止したプロセスが 2 個残っています".into(),
            buttons: vec![("再開する", BandButton::ResumeLeft, true), ("そのままにする", BandButton::KeepLeft, true)],
            born: 0.0,
            closable: false,
        });
        st.config_broken("config.toml.broken-20261122-085959");
        st.diag.spi_fail_in_row = 4;
        st
    });
}

/// app・core・win・uiprobe のソースのコメント以外にある ASCII 以外の文字を、本番と同じフォントだけで描く。
/// 描けない字があればここで落ちる(画面に出していない状態の文言・失敗の文も含めて豆腐を探す)
#[test]
fn every_non_ascii_char_in_sources_has_a_glyph() {
    fn walk(dir: &std::path::Path, out: &mut std::collections::BTreeSet<char>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                for line in std::fs::read_to_string(&p).unwrap().lines() {
                    let code = line.split("//").next().unwrap_or("");
                    out.extend(code.chars().filter(|c| !c.is_ascii()));
                }
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut chars = std::collections::BTreeSet::new();
    for d in ["procsight-app/src", "procsight-core/src", "procsight-win/src", "procsight-uiprobe/src"] {
        walk(&root.join(d), &mut chars);
    }
    assert!(chars.len() > 300, "日本語の文言を集められていない: {}", chars.len());
    let text: String = chars.into_iter().collect();
    let mut h = Harness::builder()
        .with_size([900.0, 900.0])
        .wgpu()
        .with_context_setup(|ctx| {
            let (bytes, index) = font();
            ctx.set_fonts(fonts::definitions(bytes.clone(), *index));
        })
        .build_ui(move |ui| {
            ui.add(egui::Label::new(text.as_str()).wrap());
        });
    h.run_steps(2);
    let img = h.render().expect("描けない");
    img.save(out_dir().join("glyphs-all.png")).unwrap();
}

/// パレットを開いた状態(query を入れて開く。shoot はキーを送れないので State で作る)
fn palette_open(query: &str) -> impl Fn() -> State + '_ {
    move || {
        let mut st = demo::state();
        st.palette.open = true;
        st.palette.query = query.into();
        st
    }
}

#[test]
fn palette_png() {
    shoot("palette-empty", BIG, palette_open(""));
    shoot("palette-note", BIG, palette_open("note"));
    shoot("palette-nothing", BIG, palette_open("zzzzqqq"));
    // 一致が多い(50 件で切る)。一覧のスクロールと件数の文
    shoot("palette-many", BIG, palette_open("e"));
    shoot_stress("palette", palette_open("note"));
    shoot_stress("palette-empty", palette_open(""));
}

/// 長い名前(長いサービスの表示名と exe 名)が右の札に重ならず切り詰められる
#[test]
fn palette_long_names_png() {
    let make = || {
        let mut st = palette_open("long")();
        let mut snap = (*st.snap).clone();
        let r = snap.rows.iter_mut().find(|r| r.name == "notepad.exe").unwrap();
        r.name = "very-long-process-name-for-testing-the-command-palette-row-truncation.exe".into();
        st.snap = Arc::new(snap);
        if let Load::Ready(v) = &mut st.services.list {
            let mut s = v[0].clone();
            s.name = "VeryLongServiceNameUsedOnlyInThisTest_1234567890".into();
            s.display_name = "とても長い表示名のサービス Long Display Name For Truncation Check In The Palette".into();
            v.push(s);
        }
        st
    };
    shoot("palette-long", BIG, make);
    shoot_stress("palette-long", make);
}

#[test]
fn shortcuts_png() {
    let make = || {
        let mut st = demo::state();
        st.palette.help_open = true;
        st
    };
    shoot("shortcuts", BIG, make);
    shoot_stress("shortcuts", make);
}

fn open_palette(h: &mut Harness<'static, State>) {
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run_steps(2);
    assert!(h.state().palette.open);
}

fn type_text(h: &mut Harness<'static, State>, s: &str) {
    h.event(egui::Event::Text(s.into()));
    h.run_steps(2);
    assert_eq!(h.state().palette.query, s, "入力欄に字が届いていない");
}

/// Ctrl+K で開き、「サービス」と打って Enter でサービスのタブへ移って閉じる
#[test]
fn palette_enter_switches_tab_and_closes() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(1);
    open_palette(&mut h);
    type_text(&mut h, "サービス");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Services);
    assert!(!h.state().palette.open);
    assert!(h.state().dialog.is_none());
}

/// プロセス名を打って Enter で、プロセスのタブのその行が選ばれる
#[test]
fn palette_enter_jumps_to_process() {
    let mut st = demo::state();
    st.tab = Tab::Performance;
    let id = target(&st, "notepad.exe").id;
    let mut h = harness(st, true, BIG);
    h.run_steps(1);
    open_palette(&mut h);
    type_text(&mut h, "notepad");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().tab, Tab::Processes);
    assert_eq!(h.state().procs.single(), Some(id));
    assert!(!h.state().palette.open);
}

/// ↓ で 2 つめ(パフォーマンスのタブ)に動かして Enter。Esc では何もせずに閉じる
#[test]
fn palette_arrows_and_escape() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(1);
    open_palette(&mut h);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Performance);
    open_palette(&mut h);
    type_text(&mut h, "サービス");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().palette.open);
    assert_eq!(h.state().tab, Tab::Performance);
    // 閉じた後は表のキーが効く(フォーカスを返している)
    h.state_mut().tab = Tab::Processes;
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num3);
    h.run_steps(1);
    assert_eq!(h.state().tab, Tab::Startup);
}

/// INV-5: パレットが開いている間は、行を選んでいても ↑↓・Delete・Enter で確認の窓が出ず、危ない Job も積まれない
#[test]
fn palette_keys_never_reach_the_table() {
    let dangerous = |st: &State| {
        jobs(st).into_iter().any(|j| {
            matches!(
                j,
                Job::Terminate { .. } | Job::Suspend { .. } | Job::Resume { .. } | Job::ResumeMany { .. } | Job::ServiceControl { .. } | Job::SetStartType { .. } | Job::SetPriority { .. } | Job::SetAffinity { .. } | Job::SetEfficiency { .. } | Job::SetStartup { .. } | Job::RestartElevated | Job::ReplaceEnable | Job::ReplaceDisable { .. } | Job::ReplaceRestore { .. }
            )
        })
    };
    // 開いた最初のフレーム(open を直接立てた直後)の Delete。harness は作るときにフレームを回すので、閉じて作ってから立てる
    let mut st = demo::state();
    let id = target(&st, "notepad.exe").id;
    st.procs.select_only(id);
    let mut h = harness(st, true, BIG);
    h.state_mut().palette.open = true;
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "開いた直後の Delete が表に届いた");
    // Ctrl+K で開いてから ↑↓・Delete・Enter
    let mut st = demo::state();
    st.procs.select_only(id);
    let mut h = harness(st, true, BIG);
    h.run_steps(1);
    // 閉じている間の Delete は確認の窓を出す(テストが空振りでないことの確かめ)
    h.key_press(egui::Key::Delete);
    h.run_steps(1);
    assert!(h.state().dialog.is_some());
    h.state_mut().dialog = None;
    h.run_steps(1);
    open_palette(&mut h);
    for k in [egui::Key::ArrowDown, egui::Key::ArrowUp, egui::Key::Delete, egui::Key::PageDown, egui::Key::Home, egui::Key::End] {
        h.key_press(k);
        h.run_steps(1);
        assert!(h.state().dialog.is_none(), "{k:?} が表に届いた");
    }
    assert_eq!(h.state().procs.single(), Some(id), "表の選択が動いた");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(!dangerous(h.state()));
    assert!(!h.state().palette.open);
}

/// マウスで行を押しても実行して閉じる(選んでいない行)
#[test]
fn palette_click_runs_row() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(1);
    open_palette(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "ファイルを掴んでいるプロセスを探す").click();
    h.run_steps(2);
    assert!(h.state().locks.open);
    assert!(!h.state().palette.open);
}

/// 「管理者で再起動…」は確認の窓を出すだけで、Job は積まない
#[test]
fn palette_restart_elevated_only_opens_dialog() {
    let mut st = demo::state();
    st.env.elevated = false;
    let mut h = harness(st, true, BIG);
    h.run_steps(1);
    open_palette(&mut h);
    type_text(&mut h, "管理者");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(matches!(h.state().dialog, Some(Dialog::RestartElevated { .. })));
    assert!(!jobs(h.state()).iter().any(|j| matches!(j, Job::RestartElevated)));
}

/// F1 で一覧を開き、Esc で閉じる。パレットから「キーボードのショートカット」でも開く
#[test]
fn shortcuts_open_and_close() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(1);
    h.key_press(egui::Key::F1);
    h.run_steps(2);
    assert!(h.state().palette.help_open);
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    assert!(!h.state().palette.help_open);
    open_palette(&mut h);
    type_text(&mut h, "ショートカット");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().palette.help_open && !h.state().palette.open);
}

/// Ctrl+1〜4 でタブが変わる(AC-53)
#[test]
fn ctrl_numbers_switch_tabs() {
    let mut h = harness(demo::state(), false, BIG);
    h.run_steps(1);
    for (k, t) in [(egui::Key::Num2, Tab::Performance), (egui::Key::Num3, Tab::Startup), (egui::Key::Num4, Tab::Services), (egui::Key::Num1, Tab::Processes)] {
        h.key_press_modifiers(egui::Modifiers::COMMAND, k);
        h.run_steps(1);
        assert_eq!(h.state().tab, t);
    }
}

/// v0.4.0: 起動時のタブは start_tab、Last なら last_tab(既定はプロセス)
#[test]
fn start_tab_from_config() {
    use procsight_core::config::{Config, StartTab};
    let at = |start_tab, last_tab| State::new(demo::env(), Config { start_tab, last_tab, ..Config::default() }).tab;
    assert_eq!(State::new(demo::env(), Config::default()).tab, Tab::Processes);
    assert_eq!(at(StartTab::Services, StartTab::Startup), Tab::Services);
    assert_eq!(at(StartTab::Last, StartTab::Startup), Tab::Startup);
    assert_eq!(at(StartTab::Last, StartTab::Last), Tab::Processes, "壊れた値でもプロセスに落ちる");
    for t in Tab::ALL {
        assert_eq!(Tab::of_start(t.start_tab()), Some(t));
    }
}

/// タブが変わったら last_tab に入れて保存する。変わらない間は保存しない(毎フレーム書かない)
#[test]
fn tab_change_saves_last_tab_once() {
    use procsight_core::config::StartTab;
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    assert!(!h.state().config_dirty, "既定のまま(プロセス)なら書かない");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num4);
    h.run_steps(2);
    assert_eq!(h.state().cfg.last_tab, StartTab::Services);
    assert!(h.state().config_dirty);
    h.state_mut().config_dirty = false;
    h.run_steps(3);
    assert!(!h.state().config_dirty, "同じタブの間は書かない");
    // パレットなど、他の所から変わっても拾う
    h.state_mut().tab = Tab::Performance;
    h.run_steps(1);
    assert_eq!(h.state().cfg.last_tab, StartTab::Performance);
}

/// 起動したときの窓がミニなら、最初のフレームで set_mini と同じ道でミニに入る(文字の大きさが 100% 以外でも)
#[test]
fn start_mode_mini_enters_mini_once() {
    use procsight_core::config::{Config, StartMode};
    for scale in [1.0, 1.6] {
        let cfg = Config { start_mode: StartMode::Mini, font_scale: scale, ..Config::default() };
        let mut h = harness(State::new(demo::env(), cfg), true, BIG);
        h.run_steps(3);
        assert!(h.state().mini, "文字の大きさ {scale}");
        let n = h.state().actions.iter().filter(|a| **a == Action::SetMini(true)).count();
        assert_eq!(n, 1, "入るのは 1 回だけ");
    }
    // 既定(普通の表示)とトレイでは入らない
    for mode in [StartMode::Normal, StartMode::Tray] {
        let mut h = harness(State::new(demo::env(), Config { start_mode: mode, ..Config::default() }), true, BIG);
        h.run_steps(2);
        assert!(!h.state().mini);
    }
}
