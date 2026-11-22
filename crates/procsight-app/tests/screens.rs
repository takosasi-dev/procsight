//! 窓を作らずに(egui_kittest + wgpu)S1〜S8 と確認の窓をダーク・ライトで描き、target/ui-png/ に PNG で書く。
//! フォントは本番と同じ読み込み(fonts::find)だけを入れ、描けない字があれば落ちる(豆腐の検出)。
//! あわせて、確認の窓を通さずに Job が積まれないこと(INV-5)を確かめる。

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

fn font() -> &'static (Arc<Vec<u8>>, u32) {
    static F: OnceLock<(Arc<Vec<u8>>, u32)> = OnceLock::new();
    F.get_or_init(|| {
        let (bytes, loaded) = fonts::find(&[]).expect("日本語のフォントが無い");
        (Arc::new(bytes), loaded.index)
    })
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-png");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn harness(mut st: State, dark: bool, size: [f32; 2]) -> Harness<'static, State> {
    st.cfg.theme = if dark { Theme::Dark } else { Theme::Light };
    Harness::builder()
        .with_size(size)
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .wgpu()
        .with_context_setup(|ctx| {
            let (bytes, index) = font();
            ctx.set_fonts(fonts::definitions(bytes.clone(), *index));
        })
        .build_ui_state(|ui, st: &mut State| st.ui(ui), st)
}

fn save(h: &mut Harness<'_, State>, name: &str) {
    h.run_steps(3);
    let img = h.render().expect("描けない");
    img.save(out_dir().join(format!("{name}.png"))).expect("PNG を書けない");
}

/// ダークとライトの両方で 1 枚ずつ
fn shoot(name: &str, size: [f32; 2], make: impl Fn() -> State) {
    for dark in [true, false] {
        let mut h = harness(make(), dark, size);
        save(&mut h, &format!("{name}-{}", if dark { "dark" } else { "light" }));
    }
}

const BIG: [f32; 2] = [1280.0, 800.0];

fn with_tab(tab: Tab) -> State {
    let mut st = demo::state();
    st.tab = tab;
    st
}

fn target(st: &State, name: &str) -> Target {
    let r = st.snap.rows.iter().find(|r| r.name == name).unwrap();
    Target { id: r.id, name: r.name.clone() }
}

#[test]
fn s1_processes() {
    shoot("s1-processes", BIG, demo::state);
    for t in [DTab::Threads, DTab::Modules, DTab::Handles, DTab::Signature] {
        shoot(&format!("s1-detail-{}", format!("{t:?}").to_lowercase()), BIG, || {
            let mut st = demo::state();
            st.procs.detail.tab = t;
            st
        });
    }
    shoot("s1-flat-filter", BIG, || {
        let mut st = demo::state();
        st.cfg.view = procsight_core::config::View::Flat;
        st.procs.filter = "testee".into();
        st
    });
    shoot("s1-touch-free-selected", BIG, || {
        let mut st = demo::state();
        let id = target(&st, "AceGame.exe").id;
        st.procs.select_only(id);
        st
    });
    shoot("s1-columns-all", BIG, || {
        let mut st = demo::state();
        let all: Vec<_> = procsight_core::Column::ALL.to_vec();
        st.cfg.columns.set_visible_columns(&all);
        st
    });
    shoot("s1-ascending-collapsed", BIG, || {
        let mut st = demo::state();
        st.cfg.columns.sort = procsight_core::config::SortSetting { column: "name".into(), descending: false };
        st.procs.detail.collapsed = true;
        st
    });
    shoot("s1-loading", [900.0, 500.0], || {
        let mut st = State::new(demo::env(), Default::default());
        st.set_diagnostics(demo::diagnostics());
        st
    });
    shoot("s1-no-match", [900.0, 500.0], || {
        let mut st = demo::state();
        st.procs.filter = "zzzz-nothing".into();
        st
    });
}

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

#[test]
fn s1_context_menus() {
    for (name, row) in [("s1-menu", "notepad.exe"), ("s1-menu-core", "csrss.exe"), ("s1-menu-touch-free", "AceGame.exe")] {
        for dark in [true, false] {
            let mut st = demo::state();
            let id = target(&st, row).id;
            st.procs.scroll_to = Some(id);
            let mut h = harness(st, dark, BIG);
            h.run_steps(3);
            h.get_by_label(row).click_secondary();
            h.run_steps(3);
            save(&mut h, &format!("{name}-{}", if dark { "dark" } else { "light" }));
        }
    }
}

#[test]
fn s1_columns_menu() {
    for dark in [true, false] {
        let mut h = harness(demo::state(), dark, BIG);
        h.run_steps(2);
        h.get_by_label("列…").click();
        h.run_steps(3);
        save(&mut h, &format!("s1-columns-menu-{}", if dark { "dark" } else { "light" }));
    }
}

#[test]
fn s1_thousand_rows() {
    let mut st = demo::state();
    st.set_snapshot(Arc::new(demo::big_snapshot(1000)));
    assert!(st.snap.rows.len() >= 1000);
    let mut h = harness(st, true, BIG);
    h.run_steps(13);
    save(&mut h, "s1-1000-rows-dark");
    // 時間はテストを回す PC の混み具合で揺れるので assert しない。目安として出すだけ(NFR-4 の判定は release の実機)
    let mut v: Vec<f32> = h.state().frame_ms.iter().rev().take(10).copied().collect();
    v.sort_by(|a, b| a.total_cmp(b));
    println!("1,000 行の 1 フレーム(debug)の中央値: {:.1} ms", v[v.len() / 2]);
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

/// 列の順序を変えても、列ごとの幅は config のまま(egui_extras は幅を位置で覚えるので、表を作り直す。AC-4)
#[test]
fn reordering_columns_keeps_each_columns_width() {
    use procsight_core::Column;
    let mut st = demo::state();
    st.cfg.columns.widths.insert("name".into(), 300.0);
    let mut h = harness(st, true, BIG);
    h.run_steps(3);
    let s = h.state_mut();
    let mut cols = s.cfg.columns.visible_columns();
    cols.swap(0, 1); // PID を先頭に
    s.cfg.columns.set_visible_columns(&cols);
    s.procs.invalidate();
    h.run_steps(3);
    let c = &h.state().cfg.columns;
    assert_eq!(c.visible_columns()[0], Column::Pid);
    assert_eq!(c.width(Column::Name), 300.0);
    assert_eq!(c.width(Column::Pid), Column::Pid.default_width());
}

/// Ctrl+F で絞り込みの欄に入り、Esc で空になる(AC-6・AC-53)
#[test]
fn ctrl_f_then_type_then_escape_clears_filter() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(2);
    h.event(egui::Event::Text("testee".into()));
    h.run_steps(2);
    assert_eq!(h.state().procs.filter, "testee");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().procs.filter, "");
}

#[test]
fn s2_performance() {
    for (name, page, per_core) in [
        ("s2-cpu", Page::Cpu, false),
        ("s2-cpu-cores", Page::Cpu, true),
        ("s2-memory", Page::Memory, false),
        ("s2-disk", Page::Disk("0 C:".into()), false),
        ("s2-net", Page::Net("Wi-Fi".into()), false),
        ("s2-gpu", Page::Gpu("luid_0x00000000_0x0000D1A3".into()), false),
    ] {
        shoot(name, BIG, || {
            let mut st = with_tab(Tab::Performance);
            st.perf.page = page.clone();
            st.perf.per_core = per_core;
            st
        });
    }
    shoot("s2-unavailable", BIG, || {
        let mut st = with_tab(Tab::Performance);
        let mut s = demo::snapshot(61);
        s.perf.gpus = None;
        s.perf.nets = Some(Vec::new());
        st.set_snapshot(Arc::new(s));
        st.perf.page = Page::NoGpu;
        st
    });
}

#[test]
fn s3_s4() {
    shoot("s3-startup", BIG, || with_tab(Tab::Startup));
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

fn dialog(make: impl Fn(&State) -> Dialog) -> impl Fn() -> State {
    move || {
        let mut st = demo::state();
        st.dialog = Some(make(&st));
        st
    }
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
}

fn jobs(st: &State) -> Vec<&Job> {
    st.actions.iter().filter_map(|a| if let Action::Job(j) = a { Some(j) } else { None }).collect()
}

/// INV-5: Delete で確認の窓が出るだけ。既定は「やめる」で、Enter でも実行しない。Esc で閉じる
#[test]
fn delete_opens_confirm_and_enter_or_esc_does_not_terminate() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.state_mut().actions.clear();
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(matches!(h.state().dialog, Some(Dialog::Confirm { suspend: false, .. })), "確認の窓が出る");
    assert!(jobs(h.state()).iter().all(|j| !matches!(j, Job::Terminate { .. })));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(jobs(h.state()).iter().all(|j| !matches!(j, Job::Terminate { .. })), "Enter は「やめる」");
    assert!(h.state().dialog.is_none(), "Enter で「やめる」が押された");
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(h.state().dialog.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "Esc で閉じる");
    assert!(jobs(h.state()).iter().all(|j| !matches!(j, Job::Terminate { .. })));
    // 「終了する」を押したときだけ積む
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    h.get_by_label("終了する").click();
    h.run_steps(2);
    assert!(jobs(h.state()).iter().any(|j| matches!(j, Job::Terminate { tree: false, .. })));
}

/// 中核のプロセスで Delete を押すと、帯に理由が出るだけ(AC-14・E-5)
#[test]
fn delete_on_core_process_only_shows_reason() {
    let mut st = demo::state();
    let id = target(&st, "csrss.exe").id;
    st.procs.select_only(id);
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    h.state_mut().actions.clear();
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(h.state().bands.iter().any(|b| b.text.contains("中核のプロセス") || b.text.contains("確かめられない")));
    assert!(jobs(h.state()).is_empty());
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
