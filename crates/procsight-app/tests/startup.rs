//! 起動の最初(shell::run が窓を作るまでにすること)を、窓を作らずに通す。
//! v0.1.0 が書いた config.toml を本物のファイルとして読み、本番と同じ fonts::install で字を入れ、
//! 本物の環境の値(env_info)で State を作って数フレーム描く。Backend は起こさない(他のプロセスを開かない)。

use eframe::egui;
use egui_kittest::Harness;
use procsight_app::app::State;
use procsight_app::{demo, files, fonts};
use std::sync::Arc;

const V010_CONFIG: &str = r#"schema_version = 1
interval_ms = 1000
theme = "system"
font_scale = 1.0
view = "tree"
close_action = "exit"
tray_enabled = false
elevate_on_start = false
fonts = []

[taskmgr_replace]
enabled = false
previous_debugger = ""

[touch_free]
names = []
include_descendants = true

[columns]
visible = ["name", "pid", "state", "cpu", "mem", "io_read", "io_write", "gpu", "user"]

[columns.sort]
column = "cpu"
descending = true

[window]
x = 100
y = 100
width = 1100
height = 720
maximized = false
"#;

#[test]
fn v0_1_0_config_file_then_first_frames() {
    let dir = std::env::temp_dir().join(format!("procsight-startup-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), V010_CONFIG).unwrap();
    let loaded = files::load_config(&dir, 9 * 60);
    std::fs::remove_dir_all(&dir).ok();
    assert!(loaded.broken.is_none(), "v0.1.0 の設定が壊れた扱いになった");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let cfg = loaded.config;
    assert!(cfg.heat_map, "新しい項目は既定になる");

    let env = procsight_win::env_info();
    let mut st = State::new(env, cfg);
    let choices = st.cfg.fonts.clone();
    let installed = Arc::new(std::sync::Mutex::new(None));
    let slot = installed.clone();
    let mut h = Harness::builder()
        .with_size([1100.0, 720.0])
        .wgpu()
        .with_context_setup(move |ctx: &egui::Context| {
            *slot.lock().unwrap() = fonts::install(ctx, &choices);
        })
        .build_ui_state(|ui, st: &mut State| st.ui(ui), {
            // 最初の Snapshot が届く前の 1 フレーム目を含めて描く
            st.font = None;
            st
        });
    h.run_steps(3);
    assert!(installed.lock().unwrap().is_some(), "本番と同じ字の読み込みで、日本語のフォントが見つからない");
    // 最初の Snapshot が届いた後
    h.state_mut().set_snapshot(Arc::new(demo::snapshot(1)));
    h.run_steps(5);
    h.state_mut().set_snapshot(Arc::new(demo::snapshot(2)));
    h.run_steps(5);
    assert!(h.state().frame_ms.len() >= 10);
}
