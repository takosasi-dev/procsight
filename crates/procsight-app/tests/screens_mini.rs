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
use procsight_core::config::{CpuScale, MiniItems, SpeedUnit};
use procsight_core::{PerfSample, ProcState};
use std::sync::Arc;

fn mini_state() -> State {
    let mut st = demo::state();
    st.set_mini(true);
    st.actions.clear();
    st
}

/// 描き手(wgpu)を作るテストは 1 つずつ走らせる。
/// ponytail: 並行で描き手を多く作ると、まれにプロセスごと 0xc0000005 で落ちた(並行 10 回に 1 回ほど・1 本のスレッドでは 10 回とも通過)。原因が wgpu か描き手の側で分かれば外す
fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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

// ---------------------------------------------------------------- v0.4.0 の設定(担当 I)

/// 出す物を絞った形(行の名前は widget_info の「名前 値」で見る)
fn mini_with(items: MiniItems) -> State {
    let mut st = mini_state();
    st.cfg.mini_items = items;
    st
}

const NONE: MiniItems = MiniItems { cpu: false, mem: false, gpu: false, disk: false, net_recv: false, net_send: false, top_process: false };

#[test]
fn mini_items_choose_the_rows() {
    let _gpu = gpu_lock();
    use egui_kittest::kittest::Queryable;
    let top = mini_state().perf.tops.cpu[0].name.clone();
    for (items, want, not) in [
        (MiniItems { top_process: true, ..NONE }, vec![format!("上位 {top}")], vec!["CPU ", "メモリ ", "受信 "]),
        (MiniItems { cpu: true, ..NONE }, vec!["CPU ".to_string()], vec!["上位 ", "メモリ ", "送信 "]),
        (MiniItems { net_recv: true, net_send: true, ..NONE }, vec!["受信 ".into(), "送信 ".into()], vec!["上位 ", "CPU ", "GPU "]),
    ] {
        let mut h = harness(mini_with(items.clone()), true, [MINI_SIZE.x, MINI_SIZE.y]);
        h.run_steps(2);
        for w in &want {
            assert!(h.query_by_label_contains(w).is_some(), "{items:?} で {w} が無い");
        }
        for n in not {
            assert!(h.query_by_label_contains(n).is_none(), "{items:?} で {n} が出た");
        }
    }
}

/// グラフの長さ 300: ミニを描くだけで輪の長さが合う(普通の表示の perf を通らなくても)
#[test]
fn mini_follows_graph_points() {
    let _gpu = gpu_lock();
    let mut st = mini_state();
    st.cfg.graph_points = 300;
    let mut h = harness(st, true, [MINI_SIZE.x, MINI_SIZE.y]);
    h.run_steps(2);
    assert_eq!(h.state().perf.points(), 300);
    assert_eq!(h.state().perf.cpu.len(), 60, "伸ばしても今までの 60 回分は残る");
}

/// グラフの長さ 300。full なら 300 回分まで溜めた形(set_points の後に積む)、そうでなければ 60 回分だけ(右に寄る)
fn mini_300(full: bool) -> State {
    let mut st = mini_state();
    st.cfg.graph_points = 300;
    st.perf.set_points(300);
    if full {
        for seq in 61..=360 {
            st.set_snapshot(Arc::new(demo::snapshot(seq)));
        }
    }
    st
}

fn mini_bits() -> State {
    let mut st = mini_state();
    st.cfg.speed_unit = SpeedUnit::Bits;
    st
}

fn net_only() -> State {
    mini_with(MiniItems { net_recv: true, net_send: true, ..NONE })
}

fn cpu_mem_top() -> State {
    mini_with(MiniItems { cpu: true, mem: true, top_process: true, ..NONE })
}

#[test]
fn mini_view_v040_settings() {
    let _gpu = gpu_lock();
    shoot("mini-300", [MINI_SIZE.x, MINI_SIZE.y], || mini_300(true));
    shoot("mini-300-partial", [MINI_SIZE.x, MINI_SIZE.y], || mini_300(false));
    shoot("mini-bits", [MINI_SIZE.x, MINI_SIZE.y], mini_bits);
    shoot("mini-percore", [MINI_SIZE.x, MINI_SIZE.y], || {
        let mut st = mini_state();
        st.cfg.cpu_scale = CpuScale::PerCore;
        st
    });
    shoot("mini-top-only", [MINI_SIZE.x, MINI_SIZE.y], || mini_with(MiniItems { top_process: true, ..NONE }));
    shoot("mini-cpu-only", [MINI_SIZE.x, MINI_SIZE.y], || mini_with(MiniItems { cpu: true, ..NONE }));
    shoot("mini-net-only", [MINI_SIZE.x, MINI_SIZE.y], net_only);
    shoot("mini-cpu-mem-top", [MINI_SIZE.x, MINI_SIZE.y], cpu_mem_top);
    // 崩れやすい条件(ダーク): ビットで最小の大きさ・文字 160%、行を減らして最小の大きさで 160%・入る大きさで 160%
    type Make = fn() -> State;
    let cases: [(&str, Vec2, f32, Make); 4] = [
        ("bits-min", MINI_MIN, 1.0, mini_bits),
        ("bits-scale160", MINI_SIZE, 1.6, mini_bits),
        ("net-only-min160", MINI_MIN, 1.6, net_only),
        ("cpu-mem-top-scale160", MINI_SIZE, 1.6, cpu_mem_top),
    ];
    for (suffix, pts, scale, make) in cases {
        let mut st = make();
        st.cfg.font_scale = scale;
        let mut h = harness(st, true, [pts.x * scale, pts.y * scale]);
        save(&mut h, &format!("mini-{suffix}"));
    }
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
    let _gpu = gpu_lock();
    shoot("mini", [MINI_SIZE.x, MINI_SIZE.y], mini_state);
    shoot("mini-360x300", [360.0, 300.0], mini_state);
}

/// 崩れやすい条件(ダーク): 最小の大きさ、文字 80%・160%(窓の点の大きさは変わらないので、実寸は文字の倍率を掛けた大きさ)、
/// 最小の大きさで 160%、さらに本物より厳しい「実寸 300×230 のまま 160%」
#[test]
fn mini_view_stress() {
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
