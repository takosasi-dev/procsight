//! S3 スタートアップ・S4 サービス・S5〜S8 の浮いた窓・確認の窓(担当 C)を、ダーク・ライトで PNG に描いて確かめる。
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
fn s3_s4() {
    shoot("s3-startup", BIG, || with_tab(Tab::Startup));
    // 行の高さ「詰める」(18)でも札が切れない
    shoot("s3-startup-compact", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Startup);
        st.cfg.density = procsight_core::config::Density::Compact;
        st
    });
    shoot("s4-services-comfortable", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Services);
        st.cfg.density = procsight_core::config::Density::Comfortable;
        st
    });
    shoot("s3-startup-empty", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Startup);
        st.startup.items = Load::Ready(Vec::new());
        st
    });
    shoot("s4-services", BIG, || {
        let mut st = with_tab(Tab::Services);
        st.services.pending.insert("Audiosrv".into(), "Windows Audio".into());
        st
    });
    shoot("s4-services-failed", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Services);
        st.services.list = Load::Failed(OpError::Win32(5));
        st
    });
    shoot("s4-services-loading", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Services);
        st.services.list = Load::Loading;
        st
    });
    shoot("s4-services-nomatch", [900.0, 400.0], || {
        let mut st = with_tab(Tab::Services);
        st.services.filter = "zzz".into();
        st
    });
}

#[test]
fn s5_s8_windows() {
    for p in [SPage::General, SPage::Look, SPage::TouchFree, SPage::Diagnostics] {
        shoot(&format!("s5-settings-{}", format!("{p:?}").to_lowercase()), BIG, || {
            let mut st = demo::state();
            st.settings.open = true;
            st.settings.page = p;
            st
        });
    }
    shoot("s7-about", BIG, || {
        let mut st = demo::state();
        st.about_open = true;
        st
    });
    shoot("s8-locks", BIG, || {
        let mut st = demo::state();
        st.locks.open = true;
        st.locks.files = vec!["C:\\Users\\user\\Documents\\報告書.docx".into()];
        st.locks.result = Load::Ready(demo::locking());
        st.locks.selected = Some(0);
        st
    });
    shoot("s8-locks-folder", [900.0, 600.0], || {
        let mut st = demo::state();
        st.locks.open = true;
        st.locks.problem = Some("フォルダは調べられません。中のファイルを選んでください");
        st
    });
}

#[test]
fn dialogs() {
    shoot("d1-terminate", BIG, dialog(|st| Dialog::Confirm { suspend: false, targets: vec![target(st, "notepad.exe"), target(st, "slowapp.exe")], opened: false }));
    shoot("d1-suspend", BIG, dialog(|st| Dialog::Confirm { suspend: true, targets: vec![target(st, "notepad.exe")], opened: false }));
    shoot("d2-tree", BIG, dialog(|st| {
        let root = target(st, "procsight-testee.exe");
        let kill: Vec<Target> = st.snap.rows.iter().filter(|r| r.name == "procsight-testee.exe").map(|r| Target { id: r.id, name: r.name.clone() }).collect();
        let _ = root;
        Dialog::Tree { kill, excluded: vec![(target(st, "AceGame.exe"), Refusal::TouchFree)], opened: false }
    }));
    shoot("d3-affinity", BIG, dialog(|st| Dialog::Affinity { target: target(st, "notepad.exe"), state: Load::Ready((0x0F, 0xFF)), mask: 0x0F, opened: false }));
    shoot("d3-affinity-none", [900.0, 600.0], dialog(|st| Dialog::Affinity { target: target(st, "notepad.exe"), state: Load::Ready((0, 0xFF)), mask: 0, opened: false }));
    shoot("d4-tree-result", BIG, dialog(|st| Dialog::TreeResult {
        results: vec![(target(st, "notepad.exe"), Ok(())), (target(st, "slowapp.exe"), Err(OpError::Win32(5)))],
    }));
    shoot("d5-service-stop", BIG, dialog(|_| Dialog::Service {
        name: "Audiosrv".into(),
        display_name: "Windows Audio".into(),
        action: ServiceAction::Stop,
        deps: Load::Ready(vec![("AudioEndpointBuilder".into(), "Windows Audio Endpoint Builder".into())]),
        opened: false,
    }));
    shoot("d6-start-type", BIG, dialog(|_| Dialog::StartType {
        name: "Spooler".into(),
        display_name: "Print Spooler".into(),
        current: Some(StartType::Auto),
        choice: StartType::Disabled,
        opened: false,
    }));
    shoot("d7-exit-suspended", BIG, dialog(|st| Dialog::ExitSuspended { targets: vec![target(st, "batch-job.exe")], opened: false }));
    shoot("d8-restart", BIG, dialog(|_| Dialog::RestartElevated { opened: false }));
    shoot("d9-replace", BIG, dialog(|_| Dialog::ReplaceOn { current: Some("\"C:\\Tools\\procexp64.exe\"".into()), opened: false }));
    // 30 個(20 行を超えるのでカードの中でスクロール)
    shoot("d1-terminate-many", BIG, dialog(many_targets));
}

/// shoot_stress と同じ 3 枚(狭い・80%・160%)。ただし 2 コマ描いて文字の大きさが効いてから open で窓を開く
fn shoot_stress_late(name: &str, open: impl Fn(&mut State)) {
    for (suffix, size, scale) in [("narrow", [760.0, 520.0], 1.0), ("scale80", BIG, 0.8), ("scale160", BIG, 1.6)] {
        let mut st = demo::state();
        st.cfg.font_scale = scale;
        let mut h = harness(st, true, size);
        h.run_steps(2);
        open(h.state_mut());
        save(&mut h, &format!("{name}-{suffix}"));
    }
}

fn many_targets(st: &State) -> Dialog {
    Dialog::Confirm { suspend: false, targets: st.snap.rows.iter().take(30).map(|r| Target { id: r.id, name: r.name.clone() }).collect(), opened: false }
}

/// 崩れやすい条件(狭い窓・文字 80%・160%)。浮いた窓と確認の窓がはみ出さず、ボタンが切れないこと
#[test]
fn stress() {
    shoot_stress("s3-startup", || with_tab(Tab::Startup));
    shoot_stress("s4-services", || {
        let mut st = with_tab(Tab::Services);
        st.services.pending.insert("Audiosrv".into(), "Windows Audio".into());
        st
    });
    shoot_stress("s5-settings-look", || {
        let mut st = demo::state();
        st.settings.open = true;
        st.settings.page = SPage::Look;
        st
    });
    // 浮いた窓は、文字の大きさが効いた後に開く(本番と同じ順。最初の 1 枚目で開くと、拡大の前の位置に置かれる)
    shoot_stress_late("s5-settings-look-opened", |st| {
        st.settings.open = true;
        st.settings.page = SPage::Look;
    });
    shoot_stress_late("s5-settings-general", |st| st.settings.open = true);
    shoot_stress_late("s5-settings-touchfree", |st| {
        st.settings.open = true;
        st.settings.page = SPage::TouchFree;
    });
    shoot_stress_late("s7-about", |st| st.about_open = true);
    shoot_stress_late("s8-locks", |st| {
        st.locks.open = true;
        st.locks.files = vec!["C:\\Users\\user\\Documents\\報告書.docx".into()];
        st.locks.result = Load::Ready(demo::locking());
    });
    shoot_stress("d1-terminate", dialog(many_targets));
    shoot_stress("d2-tree", dialog(|st| {
        let kill: Vec<Target> = st.snap.rows.iter().filter(|r| r.name == "procsight-testee.exe").map(|r| Target { id: r.id, name: r.name.clone() }).collect();
        Dialog::Tree { kill, excluded: vec![(target(st, "AceGame.exe"), Refusal::TouchFree)], opened: false }
    }));
    shoot_stress("d9-replace", dialog(|_| Dialog::ReplaceOn { current: Some("\"C:\\Tools\\procexp64.exe\"".into()), opened: false }));
}

/// 狭い窓・文字 160% の確認の窓でも、実行と「やめる」のボタンが画面の中にある。既定は「やめる」のまま
#[test]
fn dialog_buttons_stay_on_screen() {
    let mut st = demo::state();
    st.cfg.font_scale = 1.6;
    st.dialog = Some(many_targets(&st));
    let mut h = harness(st, true, [760.0, 520.0]);
    h.run_steps(3);
    let screen = h.ctx.content_rect();
    for label in ["終了する", "やめる"] {
        let r = h.get_by_label(label).rect();
        assert!(screen.contains_rect(r), "{label} が画面の外: {r:?} / {screen:?}");
    }
    h.state_mut().actions.clear();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "Enter で「やめる」が押された");
    assert!(jobs(h.state()).iter().all(|j| !matches!(j, Job::Terminate { .. })));
}

/// 設定を開いたまま文字を 160% にしても、窓を狭めても、設定の窓が画面の中に収まる(本番で起きる順)
#[test]
fn settings_window_stays_on_screen_after_zoom_and_resize() {
    let mut st = demo::state();
    st.settings.open = true;
    st.settings.page = SPage::Look;
    let mut h = harness(st, true, BIG);
    h.run_steps(3);
    h.state_mut().cfg.font_scale = 1.6;
    h.run_steps(4);
    let check = |h: &Harness<'static, State>, when: &str| {
        let screen = h.ctx.content_rect();
        for label in ["ゆったり", "使用量で色を塗る", "Windows に合わせる"] {
            let r = h.get_by_label(label).rect();
            assert!(screen.contains_rect(r), "{when}: {label} が画面の外: {r:?} / {screen:?}");
        }
    };
    check(&h, "160%");
    save(&mut h, "s5-settings-look-zoomed-scale160");
    h.state_mut().cfg.font_scale = 1.0;
    h.run_steps(3);
    h.set_size(egui::vec2(760.0 + KITTEST_MARGIN * 2.0, 520.0 + KITTEST_MARGIN * 2.0));
    h.run_steps(4);
    check(&h, "狭めた後");
    save(&mut h, "s5-settings-look-shrunk-narrow");
}

/// 設定の「行の高さ」を押すと cfg.density が変わり、保存の印が立つ。「使用量で色を塗る」も同じ
#[test]
fn settings_density_and_heat_map_save() {
    use procsight_core::config::Density;
    let mut st = demo::state();
    st.settings.open = true;
    st.settings.page = SPage::Look;
    let heat_before = st.cfg.heat_map;
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    assert_eq!(h.state().cfg.density, Density::Normal);
    h.state_mut().config_dirty = false;
    h.get_by_label("ゆったり").click();
    h.run_steps(2);
    assert_eq!(h.state().cfg.density, Density::Comfortable);
    assert!(h.state().config_dirty, "保存の印");
    h.state_mut().config_dirty = false;
    h.get_by_label("詰める").click();
    h.run_steps(2);
    assert_eq!(h.state().cfg.density, Density::Compact);
    assert!(h.state().config_dirty);
    h.state_mut().config_dirty = false;
    h.get_by_label("使用量で色を塗る").click();
    h.run_steps(2);
    assert_eq!(h.state().cfg.heat_map, !heat_before);
    assert!(h.state().config_dirty);
}
