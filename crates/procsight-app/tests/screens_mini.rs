//! ミニ表示・常に手前・設定の「全般」(担当 E、v0.3.0)を PNG に描き、窓の命令の関数と Ctrl+M・普通の表示への戻りを確かめる。
//! 窓は作らない(窓の命令は純粋な関数 mini::window_commands の並びで確かめる。INTERFACES §6.3)。

mod common;

use common::*;
use eframe::egui;
use egui::{Pos2, Vec2, ViewportCommand, WindowLevel};
use procsight_app::app::{Action, BandButton, BandKey, BandKind, State};
use procsight_app::demo;
use procsight_app::dialogs::Dialog;
use procsight_app::mini::{self, MINI_MIN, MINI_SIZE, NORMAL_MIN, WindowChange, WindowFacts, window_commands};
use procsight_app::settings::SPage;
use procsight_core::{PerfSample, ProcState};
use std::sync::Arc;

fn mini_state() -> State {
    let mut st = demo::state();
    st.set_mini(true);
    st.actions.clear();
    st
}

fn facts() -> WindowFacts {
    WindowFacts { pos: Some(Pos2::new(120.0, 80.0)), size: Vec2::new(1180.0, 760.0), maximized: false, zoom: 1.0, mini: None }
}

// ---------------------------------------------------------------- 窓の命令

#[test]
fn defaults_send_no_window_commands() {
    // v0.2.0 と同じに起動する: 常に手前オフでは起動時の命令が 0 個
    assert!(window_commands(WindowChange::Startup { always_on_top: false }, &facts()).is_empty());
    assert_eq!(window_commands(WindowChange::Startup { always_on_top: true }, &facts()), vec![ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)]);
    assert_eq!(window_commands(WindowChange::AlwaysOnTop(true), &facts()), vec![ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)]);
    assert_eq!(window_commands(WindowChange::AlwaysOnTop(false), &facts()), vec![ViewportCommand::WindowLevel(WindowLevel::Normal)]);
}

#[test]
fn enter_lowers_min_before_shrinking_and_exit_restores_in_order() {
    let f = facts();
    let enter = window_commands(WindowChange::EnterMini, &f);
    assert_eq!(enter, vec![ViewportCommand::MinInnerSize(MINI_MIN), ViewportCommand::InnerSize(MINI_SIZE)]);
    let exit = window_commands(WindowChange::ExitMini, &f);
    assert_eq!(
        exit,
        vec![ViewportCommand::InnerSize(f.size), ViewportCommand::MinInnerSize(NORMAL_MIN), ViewportCommand::OuterPosition(f.pos.unwrap())],
        "大きさを戻してから最小を上げ、元の位置へ"
    );
}

#[test]
fn maximized_is_released_on_enter_and_restored_last_on_exit() {
    let f = WindowFacts { maximized: true, ..facts() };
    let enter = window_commands(WindowChange::EnterMini, &f);
    assert_eq!(enter[0], ViewportCommand::Maximized(false), "最大化を先に外す");
    let exit = window_commands(WindowChange::ExitMini, &f);
    assert_eq!(exit.first(), Some(&ViewportCommand::InnerSize(f.size)), "最大化の前の大きさに戻す");
    assert_eq!(exit.last(), Some(&ViewportCommand::Maximized(true)));
}

#[test]
fn exit_min_size_matches_startup_at_any_font_scale_and_mini_place_is_reused() {
    // 文字 160% でも、最小の実寸は起動時(640×400)と同じ
    let f = WindowFacts { zoom: 1.6, pos: None, ..facts() };
    let exit = window_commands(WindowChange::ExitMini, &f);
    assert_eq!(exit, vec![ViewportCommand::InnerSize(f.size), ViewportCommand::MinInnerSize(NORMAL_MIN / 1.6)]);
    // 前にミニを置いた場所(別の画面など)に出す。最小より小さい値は最小に
    let place = (Pos2::new(-1800.0, 40.0), Vec2::new(100.0, 400.0));
    let enter = window_commands(WindowChange::EnterMini, &WindowFacts { mini: Some(place), ..facts() });
    assert_eq!(enter, vec![
        ViewportCommand::MinInnerSize(MINI_MIN),
        ViewportCommand::InnerSize(Vec2::new(MINI_MIN.x, 400.0)),
        ViewportCommand::OuterPosition(place.0)
    ]);
}

#[test]
fn geometry_is_written_again_only_after_the_window_came_back() {
    let f = facts();
    assert!(!mini::geometry_settled(&f, MINI_SIZE, false), "まだ小さい");
    assert!(mini::geometry_settled(&f, f.size + Vec2::splat(0.5), false));
    let m = WindowFacts { maximized: true, ..f };
    assert!(!mini::geometry_settled(&m, f.size, false), "最大化に戻るまで待つ");
    assert!(mini::geometry_settled(&m, Vec2::new(1920.0, 1040.0), true));
}

// ---------------------------------------------------------------- キーと戻り

#[test]
fn ctrl_m_toggles_mini_and_queues_set_mini() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.state_mut().actions.clear();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::M);
    h.run_steps(2);
    assert!(h.state().mini);
    assert!(h.state().actions.contains(&Action::SetMini(true)));
    h.state_mut().actions.clear();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::M);
    h.run_steps(2);
    assert!(!h.state().mini);
    assert!(h.state().actions.contains(&Action::SetMini(false)));
}

#[test]
fn dialog_or_button_band_or_palette_returns_to_normal_view() {
    // D7 などの確認の窓
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.run_steps(2);
    assert!(h.state().mini);
    let t = target(h.state(), "batch-job.exe");
    h.state_mut().dialog = Some(Dialog::ExitSuspended { targets: vec![t], opened: false });
    h.run_steps(2);
    assert!(!h.state().mini, "確認の窓が出たら普通の表示");
    assert!(h.state().actions.contains(&Action::SetMini(false)));
    // ボタン付きの帯(ボタンの無い帯ではミニのまま)
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.state_mut().band(BandKind::Ok, "保存しました");
    h.run_steps(2);
    assert!(h.state().mini);
    h.state_mut().push_band(procsight_app::app::Band {
        kind: BandKind::Warn,
        key: BandKey::SuspendedLeft,
        text: "前回一時停止したプロセスが 1 個残っています".into(),
        buttons: vec![("再開する", BandButton::ResumeLeft, true)],
        born: 0.0,
        closable: false,
    });
    h.run_steps(2);
    assert!(!h.state().mini, "ボタン付きの帯は小さな窓では押せない");
    // Ctrl+K(見えないパレットが開いたまま Ctrl+M が効かなくなるのを防ぐ)
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run_steps(2);
    assert!(!h.state().mini && h.state().palette.open);
}

#[test]
fn restore_button_and_top_process_return_to_normal() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.run_steps(2);
    h.get_by_label("元に戻す").click();
    h.run_steps(2);
    assert!(!h.state().mini);
    // 上位のプロセスを押すと、その行へ
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.run_steps(2);
    let top = h.state().perf.tops.cpu[0].clone();
    h.get_by_label(&format!("上位 {}", top.name)).click();
    h.run_steps(2);
    assert!(!h.state().mini);
    assert_eq!(h.state().tab, procsight_app::app::Tab::Processes);
    assert_eq!(h.state().procs.selected, vec![top.id], "一覧のその行を選ぶ");
    // 「手前」は常に手前の設定を切り替える
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    h.run_steps(2);
    h.get_by_label("手前").click();
    h.run_steps(2);
    assert!(h.state().cfg.always_on_top && h.state().mini);
    assert!(h.state().actions.contains(&Action::SetAlwaysOnTop(true)));
}

#[test]
fn double_click_on_a_metric_row_returns_to_normal() {
    let mut h = harness(mini_state(), true, [300.0, 230.0]);
    // kittest は押す・離すを 1 フレーム(0.25 秒)ずつ進めるので、ダブルクリックの間を広げる
    h.ctx.options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    h.run_steps(2);
    // 1 つめの行(CPU)の上。kittest の外の余白 8 と窓の余白 8 の内側
    let pos = egui::pos2(KITTEST_MARGIN + 8.0 + 30.0, KITTEST_MARGIN + 8.0 + 10.0);
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    assert!(h.state().mini, "動かしただけでは戻らない");
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(2);
    assert!(!h.state().mini);
}

#[test]
fn top_process_skips_idle_and_gone_rows() {
    let st = demo::state();
    let top = &st.perf.tops.cpu[0];
    assert_ne!(top.id.pid, 0, "System Idle Process は除く");
    assert!(st.snap.rows.iter().any(|r| r.id == top.id && r.is_gone == 0));
}

// ---------------------------------------------------------------- PNG

/// ミニの大きさで、ダーク・ライト。300×230(入ったときの大きさ)と 360×300(少し広げた形)
#[test]
fn mini_view() {
    shoot("mini", [MINI_SIZE.x, MINI_SIZE.y], mini_state);
    shoot("mini-360x300", [360.0, 300.0], mini_state);
}

/// 崩れやすい条件(ダーク): 最小の大きさ、文字 80%・160%(窓の点の大きさは変わらないので、実寸は文字の倍率を掛けた大きさ)、
/// 最小の大きさで 160%、さらに本物より厳しい「実寸 300×230 のまま 160%」
#[test]
fn mini_view_stress() {
    for (suffix, pts, scale) in [
        ("min", MINI_MIN, 1.0),
        ("scale80", MINI_SIZE, 0.8),
        ("scale160", MINI_SIZE, 1.6),
        ("min160", MINI_MIN, 1.6),
        ("tiny160", MINI_SIZE / 1.6, 1.6),
    ] {
        let mut st = mini_state();
        st.cfg.font_scale = scale;
        let mut h = harness(st, true, [pts.x * scale, pts.y * scale]);
        save(&mut h, &format!("mini-{suffix}"));
    }
}

/// 触れないプロセスが動いていて、ゲーム中の遅い更新が効いている・更新を止めている
#[test]
fn mini_view_touch_free_slow_and_paused() {
    shoot("mini-slow", [MINI_SIZE.x, MINI_SIZE.y], || {
        let mut st = mini_state();
        assert!(st.snap.rows.iter().any(|r| r.state == ProcState::TouchFree && r.is_gone == 0));
        st.cfg.slow_when_touch_free = true;
        st.cfg.always_on_top = true;
        st.update_slow_down();
        assert!(st.slow.active && st.effective_interval_ms == 2000);
        // 間隔が変わるとグラフは空から。少し伸びた形を描く
        for seq in 61..=80 {
            st.set_snapshot(Arc::new(demo::snapshot(seq)));
        }
        st
    });
    shoot("mini-paused", [MINI_SIZE.x, MINI_SIZE.y], || {
        let mut st = mini_state();
        st.set_paused(true);
        st
    });
}

/// 値が取れない(PDH・GPU・メモリが無い)。履歴も無い起動の直後
#[test]
fn mini_view_unavailable() {
    shoot("mini-unavailable", [MINI_SIZE.x, MINI_SIZE.y], || {
        let mut st = State::new(demo::env(), Default::default());
        let mut s = demo::snapshot(1);
        s.perf = PerfSample::default();
        s.rows.retain(|r| r.id.pid == 0 || r.is_gone > 0);
        st.set_snapshot(Arc::new(s));
        st.set_mini(true);
        st.actions.clear();
        st
    });
}

/// パレットの新しい項目(常に手前は今の値に「使用中」)と、ショートカットの一覧の Ctrl+M
#[test]
fn palette_items_and_shortcut_list() {
    shoot("palette-mini-items", BIG, || {
        let mut st = demo::state();
        st.cfg.always_on_top = true;
        st.palette.open = true;
        st.palette.query = "表示".into();
        st
    });
    shoot("shortcuts-v3", BIG, || {
        let mut st = demo::state();
        st.palette.help_open = true;
        st
    });
    // パレットから「ミニ表示」を実行するとミニに入る
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run_steps(2);
    h.state_mut().palette.query = "ミニ表示".into();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().mini && !h.state().palette.open);
}

fn settings_general() -> State {
    let mut st = demo::state();
    st.settings.open = true;
    st.settings.page = SPage::General;
    st
}

/// 設定の「全般」: 既定(オフ)と、遅い更新をオンにして今の間隔のほうが長いときの説明
#[test]
fn settings_general_page() {
    shoot("s5-general-v3", BIG, settings_general);
    shoot("s5-general-slow", BIG, || {
        let mut st = settings_general();
        st.cfg.slow_when_touch_free = true;
        st.cfg.always_on_top = true;
        st.cfg.interval_ms = 4000;
        st
    });
    shoot_stress("s5-general-v3", settings_general);
}

#[test]
fn settings_toggle_slow_takes_effect_at_once() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(settings_general(), true, BIG);
    h.run_steps(2);
    h.state_mut().actions.clear();
    h.get_by_label("触れないプロセスが動いている間は更新を遅くする").click();
    h.run_steps(2);
    let st = h.state();
    assert!(st.cfg.slow_when_touch_free && st.config_dirty);
    assert!(st.actions.contains(&Action::SetInterval(2000)), "触れないプロセスが動いているのですぐ遅くする");
    assert_eq!(st.cfg.interval_ms, 1000, "利用者の選んだ間隔は変えない");
}
