//! 本物の Backend の値で、画面の状態を窓を作らずに数十フレーム回す(配布物が起動直後に落ちないことの確かめ)。
//! テストを回す PC の他のプロセス(ゲームを含む)を開かないよう、このテストの exe 以外の今動いている名前を全部「触れない」にする。
//! 窓は作らない(egui_kittest は画面の外の wgpu で描く)。

use eframe::egui;
use egui_kittest::Harness;
use procsight_app::app::{Action, State, Tab};
use procsight_app::details::DTab;
use procsight_app::fonts;
use procsight_core::config::Config;
use procsight_core::rules::TouchFreeSet;
use procsight_win::{Backend, BackendConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn others_touch_free() -> TouchFreeSet {
    let me = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().to_lowercase();
    let mut names = Vec::new();
    for p in procsight_win::nt::spi::query_processes().unwrap() {
        let n = p.image_name.to_lowercase();
        if !n.is_empty() && n != me && !names.contains(&n) {
            names.push(n);
        }
    }
    TouchFreeSet { names, include_descendants: false, ..Default::default() }
}

/// shell.rs の logic と run_actions のうち、窓に関わらない所だけ
fn pump(b: &Backend, st: &mut State) {
    while let Some(r) = b.try_recv() {
        st.apply(r);
    }
    st.set_snapshot(b.snapshot());
    st.set_diagnostics(b.diagnostics());
    for a in std::mem::take(&mut st.actions) {
        match a {
            Action::Job(j) => b.submit(j),
            Action::SetInterval(ms) => b.set_interval(ms),
            Action::SetPaused(p) => b.set_paused(p),
            Action::SetTouchFree(_) | Action::SetTiming(_) | Action::Exit | Action::TrayChanged => {}
        }
    }
}

#[test]
fn real_backend_screens_do_not_panic() {
    let touch = others_touch_free();
    let mut cfg = Config { interval_ms: 500, ..Default::default() };
    cfg.touch_free.names = touch.names.clone();
    cfg.touch_free.include_descendants = false;
    let b = Backend::start(BackendConfig { interval_ms: 500, touch_free: touch }, Arc::new(|| {}));
    let st = State::new(procsight_win::env_info(), cfg);
    let (bytes, loaded) = fonts::find(&[]).expect("日本語のフォントが無い");
    let bytes = Arc::new(bytes);
    let mut h = Harness::builder()
        .with_size([1100.0, 720.0])
        .wgpu()
        .with_context_setup(move |ctx: &egui::Context| ctx.set_fonts(fonts::definitions(bytes.clone(), loaded.index)))
        .build_ui_state(|ui, st: &mut State| st.ui(ui), st);

    let run = |h: &mut Harness<'_, State>, secs: u64| {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            pump(&b, h.state_mut());
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(50));
        }
    };

    run(&mut h, 3);
    let rows = h.state().snap.rows.len();
    assert!(rows > 20, "行が {rows} しかない");
    // 自分の行(触れないにしていない)を選んで詳細の全部のタブを回す
    let me = h.state().snap.find_pid(std::process::id()).expect("自分の行").id;
    h.state_mut().procs.select_only(me);
    for tab in [DTab::General, DTab::Threads, DTab::Modules, DTab::Handles, DTab::Signature] {
        h.state_mut().procs.detail.tab = tab;
        run(&mut h, 2);
    }
    assert!(h.state().threads.contains_key(&me), "スレッドの一覧が届かない");
    for tab in [Tab::Performance, Tab::Startup, Tab::Services, Tab::Processes] {
        h.state_mut().tab = tab;
        run(&mut h, 2);
    }
    // 実データの見え方を target/ の下にだけ書く(この PC のプロセス名が写るので、公開物には使わない)
    h.state_mut().procs.detail.tab = DTab::General;
    run(&mut h, 2);
    let img = h.render().expect("描けない");
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-png");
    std::fs::create_dir_all(&dir).unwrap();
    img.save(dir.join("live-s1.png")).unwrap();
    let (touch_opens, _) = b.shutdown();
    assert_eq!(touch_opens, 0, "触れないプロセスを開いた");
}
