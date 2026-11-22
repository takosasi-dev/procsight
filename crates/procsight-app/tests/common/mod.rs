//! 画面の PNG のテストで共有する道具(担当ごとのテストのファイルから mod common で読む)。
//! 窓を作らずに(egui_kittest + wgpu)描き、target/ui-png/ に PNG で書く。フォントは本番と同じ読み込みだけを入れる。
#![allow(dead_code, unused_imports)]

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

pub fn font() -> &'static (Arc<Vec<u8>>, u32) {
    static F: OnceLock<(Arc<Vec<u8>>, u32)> = OnceLock::new();
    F.get_or_init(|| {
        let (bytes, loaded) = fonts::find(&[]).expect("日本語のフォントが無い");
        (Arc::new(bytes), loaded.index)
    })
}

pub fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-png");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// kittest は画面の周りに 8 の外の余白を置く。本番の窓と同じ大きさで描けるよう、その分を足して作り、PNG では切り落とす
pub const KITTEST_MARGIN: f32 = 8.0;

pub fn harness(mut st: State, dark: bool, size: [f32; 2]) -> Harness<'static, State> {
    st.cfg.theme = if dark { Theme::Dark } else { Theme::Light };
    Harness::builder()
        .with_size([size[0] + KITTEST_MARGIN * 2.0, size[1] + KITTEST_MARGIN * 2.0])
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .wgpu()
        .with_context_setup(|ctx| {
            let (bytes, index) = font();
            ctx.set_fonts(fonts::definitions(bytes.clone(), *index));
        })
        .build_ui_state(|ui, st: &mut State| st.ui(ui), st)
}

pub fn save(h: &mut Harness<'_, State>, name: &str) {
    h.run_steps(3);
    let img = h.render().expect("描けない");
    let m = KITTEST_MARGIN as u32;
    let (w, hh) = (img.width() - m * 2, img.height() - m * 2);
    let img = image::imageops::crop_imm(&img, m, m, w, hh).to_image();
    img.save(out_dir().join(format!("{name}.png"))).expect("PNG を書けない");
}

/// ダークとライトの両方で 1 枚ずつ
pub fn shoot(name: &str, size: [f32; 2], make: impl Fn() -> State) {
    for dark in [true, false] {
        let mut h = harness(make(), dark, size);
        save(&mut h, &format!("{name}-{}", if dark { "dark" } else { "light" }));
    }
}

/// 崩れやすい条件で 1 枚ずつ(ダークだけ): 狭い窓(最小の 640×400 に近い 760×520)、文字の大きさ 80% と 160%、狭い窓で 160%
pub fn shoot_stress(name: &str, make: impl Fn() -> State) {
    for (suffix, size, scale) in [("narrow", [760.0, 520.0], 1.0), ("scale80", BIG, 0.8), ("scale160", BIG, 1.6), ("narrow160", [760.0, 520.0], 1.6)] {
        let mut st = make();
        st.cfg.font_scale = scale;
        let mut h = harness(st, true, size);
        save(&mut h, &format!("{name}-{suffix}"));
    }
}

pub const BIG: [f32; 2] = [1280.0, 800.0];

pub fn with_tab(tab: Tab) -> State {
    let mut st = demo::state();
    st.tab = tab;
    st
}

pub fn target(st: &State, name: &str) -> Target {
    let r = st.snap.rows.iter().find(|r| r.name == name).unwrap();
    Target { id: r.id, name: r.name.clone() }
}

pub fn dialog(make: impl Fn(&State) -> Dialog) -> impl Fn() -> State {
    move || {
        let mut st = demo::state();
        st.dialog = Some(make(&st));
        st
    }
}

pub fn jobs(st: &State) -> Vec<&Job> {
    st.actions.iter().filter_map(|a| if let Action::Job(j) = a { Some(j) } else { None }).collect()
}
