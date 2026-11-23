//! S1 プロセスのタブと詳細パネル(担当 A)を、ダーク・ライトで PNG に描いて確かめる。
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

/// v0.2.0: 崩れやすい条件・塗りなし・絞り込みの書き方・行の高さ
#[test]
fn s1_wave2_looks() {
    shoot_stress("s1-processes", demo::state);
    shoot("s1-heat-off", BIG, || {
        let mut st = demo::state();
        st.cfg.heat_map = false;
        st
    });
    shoot("s1-filter-syntax", BIG, || {
        let mut st = demo::state();
        st.procs.filter = "cpu>1 mem>100mb".into();
        st
    });
    for d in [procsight_core::config::Density::Compact, procsight_core::config::Density::Comfortable] {
        shoot(&format!("s1-density-{}", format!("{d:?}").to_lowercase()), BIG, || {
            let mut st = demo::state();
            st.cfg.density = d;
            st
        });
    }
    // 「?」にマウスを置くと書き方の一覧が出る
    for dark in [true, false] {
        let mut h = harness(demo::state(), dark, BIG);
        h.run_steps(2);
        h.get_by_label("絞り込みの書き方").hover();
        h.run_steps(3);
        save(&mut h, &format!("s1-filter-help-{}", if dark { "dark" } else { "light" }));
    }
}

/// 詳細パネルの取得中・取れない(権限)・空と、長い名前の見出し、切れた名前のツールチップ
#[test]
fn s1_detail_states_and_long_names() {
    shoot("s1-detail-loading", BIG, || {
        let mut st = demo::state();
        st.procs.detail.general = Load::Loading;
        st
    });
    shoot("s1-detail-denied", BIG, || {
        let mut st = demo::state();
        st.procs.detail.general = Load::Failed(OpError::Win32(5));
        st
    });
    shoot("s1-detail-modules-empty", BIG, || {
        let mut st = demo::state();
        st.procs.detail.tab = DTab::Modules;
        st.procs.detail.modules = Load::Ready(Vec::new());
        st
    });
    // 長い名前の行を選んでも、札とタブは押し出されない(狭い窓)
    let long = || {
        let mut st = demo::state();
        let id = target(&st, "notepad.exe").id;
        let snap = Arc::make_mut(&mut st.snap);
        snap.rows.iter_mut().find(|r| r.id == id).unwrap().name = "a-very-long-process-name-for-layout-check.exe".into();
        st
    };
    let mut h = harness(long(), true, [760.0, 520.0]);
    save(&mut h, "s1-long-name-narrow");
    // 見出しの最後のタブが押せる位置にある
    h.get_by_label("署名").click();
    h.run_steps(2);
    assert_eq!(h.state().procs.detail.tab, DTab::Signature);
    // 名前の列を狭めると名前が切れ、マウスを置くと全部の名前とパスが出る
    for dark in [true, false] {
        let mut st = demo::state();
        st.cfg.view = procsight_core::config::View::Flat;
        st.cfg.columns.widths.insert("name".into(), 90.0);
        let mut h = harness(st, dark, BIG);
        h.run_steps(3);
        h.get_all_by_label_contains("AceGame").next().expect("AceGame の行がある").hover();
        h.run_steps(3);
        save(&mut h, &format!("s1-name-tooltip-{}", if dark { "dark" } else { "light" }));
    }
}

/// 見出しを押すと並べ替える。同じ列をもう一度押すと向きが変わる
#[test]
fn header_click_sorts() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.get_by_label("PID").click();
    h.run_steps(2);
    let s = &h.state().cfg.columns.sort;
    assert_eq!((s.column.as_str(), s.descending), ("pid", true));
    h.get_by_label("PID ▼").click();
    h.run_steps(2);
    let s = &h.state().cfg.columns.sort;
    assert_eq!((s.column.as_str(), s.descending), ("pid", false));
}

/// 絞り込みの書き方は AND で効く(表の行の数で確かめる)
#[test]
fn filter_syntax_narrows_rows() {
    let mut st = demo::state();
    st.cfg.view = procsight_core::config::View::Flat;
    st.procs.filter = "cpu>1 mem>100mb".into();
    let mut h = harness(st, true, BIG);
    h.run_steps(3);
    let s = h.state();
    let names: Vec<&str> = s.procs.visible.iter().map(|v| s.snap.rows[v.index].name.as_str()).collect();
    assert!(!names.is_empty());
    for v in &s.procs.visible {
        let r = &s.snap.rows[v.index];
        assert!(r.cpu_pct.unwrap() > 1.0 && r.mem.unwrap() > 100 * 1024 * 1024, "{}", r.name);
    }
}

/// System Idle Process(PID 0)は、どの列・どちら向きでも一番下(一覧でもツリーの根の並びでも)
#[test]
fn idle_process_is_always_last() {
    use procsight_core::config::{SortSetting, View};
    for view in [View::Flat, View::Tree] {
        for (col, descending) in [("cpu", true), ("cpu", false), ("name", false), ("name", true), ("pid", false), ("pid", true)] {
            let mut st = demo::state();
            st.cfg.view = view;
            st.cfg.columns.sort = SortSetting { column: col.into(), descending };
            let mut h = harness(st, true, BIG);
            h.run_steps(2);
            let s = h.state();
            let roots: Vec<u32> = s.procs.visible.iter().filter(|v| v.depth == 0).map(|v| s.snap.rows[v.index].id.pid).collect();
            assert_eq!(roots.last(), Some(&0), "{view:?} {col} desc={descending}: {roots:?}");
            if view == View::Flat {
                assert_eq!(s.snap.rows[s.procs.visible.last().unwrap().index].id.pid, 0);
            }
        }
    }
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
            // 詳細パネルの見出しにも同じ名前があるので、一番上(表の行)を押す
            h.get_all_by_label(row).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click_secondary();
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

// ---------------------------------------------------------------- v0.3.0: CPU の推移の列と CSV の保存(担当 F)

/// CPU の推移の列が見える形(一覧・CPU の順。動きのある browser.exe を選ぶ)。
/// 見本の CPU は周ごとの揺れが小さいので、さらに 60 回分、行ごとに形の違う動き(波・山・上り)を足す
fn cpu_history_state() -> State {
    let mut st = demo::state();
    for seq in 61..=120u64 {
        let mut s = demo::snapshot(seq);
        let t = (seq - 61) as f32;
        for r in s.rows.iter_mut().filter(|r| r.id.pid != 0) {
            let Some(c) = r.cpu_pct.as_mut() else { continue };
            *c = match r.name.as_str() {
                "AceGame.exe" => 30.0 + 25.0 * (t * 0.25).sin() + 10.0 * (t * 0.9).sin(),
                "procsight-testee.exe" if r.id.pid == 10_000 => (t * 1.6).min(96.0),
                "browser.exe" if r.id.pid == 8812 => if (20..28).contains(&(seq % 40)) { 45.0 } else { 3.0 + (t * 0.7).sin().abs() * 2.0 },
                "dwm.exe" => 2.0 + 1.5 * (t * 0.5).sin(),
                "svchost.exe" if seq % 15 == 0 => *c + 18.0,
                _ => *c,
            }
            .max(0.0);
        }
        st.set_snapshot(Arc::new(s));
    }
    st.cfg.view = procsight_core::config::View::Flat;
    let id = target(&st, "browser.exe").id;
    st.procs.select_only(id);
    st
}

#[test]
fn s1_cpu_history_looks() {
    shoot("s1-cpu-history", BIG, cpu_history_state);
    shoot_stress("s1-cpu-history", cpu_history_state);
    // 触れないプロセスの行を選んでも描く(SPI の値)
    shoot("s1-cpu-history-touch-free", BIG, || {
        let mut st = cpu_history_state();
        let id = target(&st, "AceGame.exe").id;
        st.procs.select_only(id);
        st
    });
    // System Idle Process(一番下)は使用量の色にしない。名前で絞って見えるところへ
    shoot("s1-cpu-history-idle", BIG, || {
        let mut st = cpu_history_state();
        st.procs.filter = "system".into();
        st
    });
    // 塗り(heat)を切ったときと、行の高さが一番低いとき
    shoot("s1-cpu-history-compact-heat-off", BIG, || {
        let mut st = cpu_history_state();
        st.cfg.heat_map = false;
        st.cfg.density = procsight_core::config::Density::Compact;
        st
    });
}

/// 推移は Snapshot の seq が変わったときだけ進み、終わった・消えた識別子は消える。最小化中の周は触らない
#[test]
fn cpu_history_follows_snapshots() {
    let mut st = demo::state();
    let browser = target(&st, "browser.exe").id;
    let game = target(&st, "AceGame.exe").id;
    let hist = |st: &State, id| st.procs.cpu_history(id).map(|h| h.collect::<Vec<f32>>());
    let h = hist(&st, browser).unwrap();
    let game_before = hist(&st, game).unwrap();
    assert_eq!(h.len(), 60, "見本は 60 回分");
    assert_eq!(h.last().copied(), st.snap.find(browser).unwrap().cpu_pct);
    assert_eq!(game_before.len(), 60, "触れないプロセスも持つ");
    // 終わった行(is_gone)の推移は持たない
    assert!(hist(&st, target(&st, "updater.exe").id).is_none());
    // 同じ seq は足さない
    st.set_snapshot(Arc::new(demo::snapshot(60)));
    assert_eq!(hist(&st, browser).unwrap(), h);
    // 最小化中の周(rows_stale)は足しも消しもしない
    let mut stale = demo::snapshot(61);
    stale.rows.clear();
    stale.rows_stale = true;
    st.procs.on_snapshot(&stale);
    assert_eq!(hist(&st, browser).unwrap(), h);
    // 一覧から消えた識別子は消える。60 回を超えたら古い方から落ちる
    let mut next = demo::snapshot(62);
    next.rows.retain(|r| r.id != browser);
    let game_now = next.find(game).unwrap().cpu_pct.unwrap();
    st.set_snapshot(Arc::new(next));
    assert!(hist(&st, browser).is_none());
    let g = hist(&st, game).unwrap();
    assert_eq!(g.len(), 60);
    assert_eq!(g[..59], game_before[1..]);
    assert_eq!(g[59], game_now);
    assert_eq!(st.procs.cpu_history_len(), st.snap.rows.iter().filter(|r| r.is_gone == 0).count());
}

/// 折れ線にマウスを置くと今・最大・平均が出る。折れ線の上を押しても行が選べる
#[test]
fn cpu_history_tooltip_and_click_selects_row() {
    for dark in [true, false] {
        let mut h = harness(cpu_history_state(), dark, BIG);
        h.run_steps(3);
        let x = h.get_by_label("CPU の推移").rect().center().x;
        let y = h.get_all_by_label("AceGame.exe").map(|n| n.rect().center().y).fold(f32::MAX, f32::min);
        let pos = egui::pos2(x, y);
        h.hover_at(pos);
        h.run_steps(3);
        save(&mut h, &format!("s1-cpu-history-tooltip-{}", if dark { "dark" } else { "light" }));
        assert!(h.query_by_label_contains("CPU の推移(直近 60 回)").is_some(), "ツールチップが出る");
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(2);
        let s = h.state();
        assert_eq!(s.procs.single(), Some(target(s, "AceGame.exe").id), "押した行が選ばれる");
    }
}

/// CPU の推移の見出しを初めて押すと、大きい順(今の CPU)に並ぶ
#[test]
fn cpu_history_header_sorts_descending_first() {
    let mut h = harness(demo::state(), true, BIG);
    h.run_steps(2);
    h.get_by_label("CPU の推移").click();
    h.run_steps(2);
    let s = &h.state().cfg.columns.sort;
    assert_eq!((s.column.as_str(), s.descending), ("cpu_history", true));
}

/// 1,000 行の全部が 60 回分の推移を持つとき(一番重い形)の 1 フレームの目安
#[test]
fn s1_thousand_rows_with_history() {
    let mut st = demo::state();
    for seq in 61..=120 {
        let mut s = demo::big_snapshot(1000);
        s.seq = seq;
        st.set_snapshot(Arc::new(s));
    }
    let id = st.snap.rows[600].id;
    assert_eq!(st.procs.cpu_history(id).unwrap().len(), 60);
    let mut h = harness(st, true, BIG);
    h.run_steps(13);
    save(&mut h, "s1-1000-rows-history-dark");
    let mut v: Vec<f32> = h.state().frame_ms.iter().rev().take(10).copied().collect();
    v.sort_by(|a, b| a.total_cmp(b));
    println!("1,000 行(全部に推移)の 1 フレーム(debug)の中央値: {:.1} ms", v[v.len() / 2]);
}

/// 「一覧を CSV で保存…」は保存先を選ぶ Job を積むだけ(本物の保存の窓はテストで出さない)
#[test]
fn csv_request_queues_pick_save_file() {
    let mut st = demo::state();
    procsight_app::export::request(&mut st);
    let js = jobs(&st);
    assert_eq!(js.len(), 1);
    let Job::PickSaveFile { suggested } = js[0] else { panic!("{:?}", js[0]) };
    assert!(suggested.starts_with("procsight-") && suggested.ends_with(".csv"), "{suggested}");
    assert_eq!(suggested.len(), "procsight-YYYYMMDD-HHMMSS.csv".len(), "{suggested}");
}

/// やめたら何も起きない。選んだら書いて成功の帯。書けなければ失敗の帯
#[test]
fn csv_on_picked_writes_file_and_shows_band() {
    let mut st = demo::state();
    procsight_app::export::on_picked(&mut st, None);
    assert!(st.bands.is_empty() && st.actions.is_empty());

    let dir = std::env::temp_dir().join(format!("procsight-f-csv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("list.csv");
    procsight_app::export::on_picked(&mut st, Some(path.to_string_lossy().into_owned()));
    let bytes = std::fs::read(&path);
    std::fs::remove_dir_all(&dir).unwrap();
    let bytes = bytes.unwrap();
    let n = st.procs.visible.len();
    assert!(n > 20, "表を描く前でも、見えている行を作ってから書く");
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.matches("\r\n").count(), n + 1, "見出し + {n} 行");
    assert!(text.contains("\"notepad.exe\""));
    let b = &st.bands[0];
    assert_eq!((b.kind, b.text.as_str()), (BandKind::Ok, format!("一覧を CSV で保存しました({n} 行)").as_str()));

    // 無いフォルダには書けない
    let bad = std::env::temp_dir().join(format!("procsight-f-missing-{}", std::process::id())).join("x.csv");
    procsight_app::export::on_picked(&mut st, Some(bad.to_string_lossy().into_owned()));
    let b = &st.bands[0];
    assert_eq!(b.kind, BandKind::Danger);
    assert!(b.text.starts_with("CSV を保存できませんでした。"), "{}", b.text);
}

// ---------------------------------------------------------------- v0.4.0: 設定を読む側(担当 H)

use procsight_core::config::{Config, CpuScale, DoubleClick, HeatFull, HeatStart, TreeStart, View};

/// 設定を変えてから見本の Snapshot を流した状態(demo::state と同じ形。最初の Snapshot の前に設定が効く)
fn state_with(edit: impl Fn(&mut Config)) -> State {
    let mut cfg = Config::default();
    cfg.touch_free.names = vec!["acegame.exe".into()];
    cfg.columns.visible.push("threads".into());
    edit(&mut cfg);
    let mut st = State::new(demo::env(), cfg);
    st.set_diagnostics(demo::diagnostics());
    let notepad = demo::snapshot(1).rows.iter().find(|r| r.name == "notepad.exe").unwrap().id;
    st.procs.select_only(notepad);
    for seq in 1..=60 {
        st.set_snapshot(Arc::new(demo::snapshot(seq)));
    }
    st.procs.detail.general = Load::Ready(demo::general());
    st
}

/// 表を描いて並びを作る(refresh は crate の中だけなので、描いて呼ばせる)
fn drawn(st: State) -> Harness<'static, State> {
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    h
}

fn visible_ids(st: &State) -> Vec<procsight_core::ProcId> {
    st.procs.visible.iter().map(|v| st.snap.rows[v.index].id).collect()
}

/// CPU の数え方「コア 1 つで 100%」: プロセスごとの数字(表・コピー・CSV・推移の文字)は ×論理 CPU の数。
/// 並べ替えと推移の輪は掛ける前のまま。絞り込みの cpu>N は表の数字と比べる
#[test]
fn cpu_per_core_scales_process_numbers_only() {
    use procsight_app::procs::cell_text;
    use procsight_core::Column;
    let base = demo::state();
    let id = target(&base, "procsight-testee.exe").id;
    let raw = base.snap.find(id).unwrap().cpu_pct.unwrap();
    let mut st = demo::state();
    st.cfg.cpu_scale = CpuScale::PerCore;
    let r = st.snap.find(id).unwrap();
    let k = st.snap.logical_cpus as f32;
    assert_eq!(k, 8.0);
    assert_eq!(cell_text(&st, r, Column::Cpu), format!("{:.1}%", raw * k));
    assert_eq!(cell_text(&st, r, Column::CpuHistory), format!("{:.1}%", raw * k));
    assert_eq!(cell_text(&base, r, Column::Cpu), format!("{raw:.1}%"), "既定は今のまま");
    assert_eq!(st.procs.cpu_history(id).unwrap().last(), Some(raw), "推移の輪は掛ける前の値");
    // 絞り込み: 掛けた値で比べる(raw × 8 は 50 を超え、raw は超えない)
    assert!(raw < 50.0 && raw * k > 50.0, "{raw}");
    let filtered = |scale: CpuScale| {
        let mut st = demo::state();
        st.cfg.cpu_scale = scale;
        st.cfg.view = View::Flat;
        st.procs.filter = "cpu>50".into();
        visible_ids(drawn(st).state())
    };
    assert!(!filtered(CpuScale::Total).contains(&id));
    assert!(filtered(CpuScale::PerCore).contains(&id));
    // 設定を変えたら、次の Snapshot を待たずに並びを作り直す
    let mut st = demo::state();
    st.cfg.view = View::Flat;
    st.procs.filter = "cpu>50".into();
    let mut h = drawn(st);
    assert!(!visible_ids(h.state()).contains(&id));
    h.state_mut().cfg.cpu_scale = CpuScale::PerCore;
    h.run_steps(2);
    assert!(visible_ids(h.state()).contains(&id));
    // 並べ替え(CPU の降順)の順は変わらない
    let order = |scale: CpuScale| {
        let mut st = demo::state();
        st.cfg.cpu_scale = scale;
        st.cfg.view = View::Flat;
        st.cfg.columns.sort = procsight_core::config::SortSetting { column: "cpu".into(), descending: true };
        visible_ids(drawn(st).state())
    };
    assert_eq!(order(CpuScale::Total), order(CpuScale::PerCore));
    // CSV も表と同じ数字
    let mut st = demo::state();
    st.cfg.cpu_scale = CpuScale::PerCore;
    st.cfg.view = View::Flat;
    st.cfg.columns.set_visible_columns(&[Column::Name, Column::Pid, Column::Cpu]);
    st.procs.filter = "pid:10000".into();
    let h = drawn(st);
    let (csv, n) = procsight_app::export::csv_text(h.state());
    assert_eq!(n, 1);
    assert!(csv.contains(&format!("\"10000\",\"{:.1}%\"", raw * k)), "{csv}");
}

#[test]
fn s1_cpu_per_core_looks() {
    let make = || {
        let mut st = cpu_history_state();
        st.cfg.cpu_scale = CpuScale::PerCore;
        st
    };
    shoot("s1-cpu-per-core", BIG, make);
    shoot_stress("s1-cpu-per-core", make);
    // 詳細のグラフの数字: CPU を使っている procsight-testee.exe(PID 10000)を選び、60 回分ためる
    let detail = || {
        let mut st = state_with(|c| c.cpu_scale = CpuScale::PerCore);
        let id = st.snap.find_pid(10_000).unwrap().id;
        st.procs.select_only(id);
        for seq in 61..=120 {
            st.set_snapshot(Arc::new(demo::snapshot(seq)));
        }
        st
    };
    shoot("s1-cpu-per-core-detail", BIG, detail);
    shoot_stress("s1-cpu-per-core-detail", detail);
    let st = detail();
    let raw = st.procs.detail.cpu.last().copied().unwrap();
    assert!(raw > 1.0);
    let h = drawn(st);
    let want = format!("{:.1}%", raw * 8.0);
    assert_eq!(h.query_all_by_label(&want).count(), 2, "表の行と詳細の CPU の数字が掛けた値 {want}");
    // 推移のツールチップの今・最大・平均も掛けた数字
    let mut h = harness(make(), true, BIG);
    h.run_steps(3);
    let x = h.get_by_label("CPU の推移").rect().center().x;
    let y = h.get_all_by_label("AceGame.exe").map(|n| n.rect().center().y).fold(f32::MAX, f32::min);
    h.hover_at(egui::pos2(x, y));
    h.run_steps(3);
    save(&mut h, "s1-cpu-per-core-tooltip-dark");
    let game = target(h.state(), "AceGame.exe").id;
    let now = h.state().procs.cpu_history(game).unwrap().last().unwrap();
    let want = format!("今 {:.1}%", now * 8.0);
    assert!(h.query_by_label_contains(&want).is_some(), "{want}");
}

/// 塗り始める所・一番濃くする所(既定の絵は s1-processes と同じ。ここは既定でない設定の絵)
#[test]
fn s1_heat_range_looks() {
    shoot("s1-heat-high-half", BIG, || {
        let mut st = demo::state();
        st.cfg.heat_start = HeatStart::High;
        st.cfg.heat_full = HeatFull::Half;
        st
    });
    shoot("s1-heat-some-near", BIG, || {
        let mut st = demo::state();
        st.cfg.heat_start = HeatStart::Some;
        st.cfg.heat_full = HeatFull::Near;
        st
    });
}

/// 行をダブルクリックする(押す・離すを 2 回)
fn double_click_row(h: &mut Harness<'_, State>, name: &str) {
    // kittest は押す・離すを 1 フレームずつ進めるので、ダブルクリックの間を広げる
    h.ctx.options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    let r = h.get_all_by_label(name).map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    // 名前の文字の少し右(開閉の印・アイコンを避ける)
    let pos = egui::pos2(r.right() + 20.0, r.center().y);
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(2);
}

/// ダブルクリック: 詳細(今の動き)・ファイルの場所(メニューと同じ Job。触れない・パスが無いなら何もしない)・何もしない(選ぶだけ)
#[test]
fn double_click_follows_setting() {
    let make = |d: DoubleClick| {
        let mut st = demo::state();
        st.cfg.double_click = d;
        st.cfg.view = View::Flat;
        st.procs.detail.collapsed = true;
        st
    };
    let opened = |st: &State| jobs(st).into_iter().filter_map(|j| if let Job::OpenLocation { path } = j { Some(path.clone()) } else { None }).collect::<Vec<_>>();

    let mut h = drawn(make(DoubleClick::Details));
    double_click_row(&mut h, "dwm.exe");
    assert_eq!(h.state().procs.single(), Some(target(h.state(), "dwm.exe").id));
    assert!(!h.state().procs.detail.collapsed, "詳細を開く");
    assert!(opened(h.state()).is_empty());

    let mut h = drawn(make(DoubleClick::OpenLocation));
    double_click_row(&mut h, "dwm.exe");
    let s = h.state();
    let dwm = target(s, "dwm.exe").id;
    let path = s.snap.find(dwm).unwrap().path().unwrap().to_string();
    assert_eq!(s.procs.single(), Some(dwm), "行は選ぶ");
    assert_eq!(opened(s), vec![path]);
    assert!(s.procs.detail.collapsed, "詳細は開かない");
    // 触れないプロセス(パスも無い)は何もしない
    let mut h = drawn(make(DoubleClick::OpenLocation));
    double_click_row(&mut h, "AceGame.exe");
    assert_eq!(h.state().procs.single(), Some(target(h.state(), "AceGame.exe").id));
    assert!(opened(h.state()).is_empty());

    let mut h = drawn(make(DoubleClick::Nothing));
    double_click_row(&mut h, "dwm.exe");
    assert_eq!(h.state().procs.single(), Some(target(h.state(), "dwm.exe").id), "選ぶだけ");
    assert!(h.state().procs.detail.collapsed && opened(h.state()).is_empty());
}

/// ツリーの最初の開き方「畳む」: 初めて見えた物だけ畳む。手で開いた物はそのまま、設定を変えても今ある物は畳み直さない
#[test]
fn tree_start_collapsed_folds_only_new_processes() {
    let st = state_with(|c| c.tree_start = TreeStart::Collapsed);
    let root = st.snap.find_pid(10_000).unwrap().id;
    assert!(st.procs.is_folded(root), "起動して見えた物は畳む");
    let expanded_rows = drawn(demo::state()).state().procs.visible.len();
    let mut h = drawn(st);
    assert!(h.state().procs.visible.len() < expanded_rows, "畳んだので行が減る");
    // 手で開いた物は、次の Snapshot でも開いたまま
    h.state_mut().procs.set_folded(root, false);
    h.state_mut().set_snapshot(Arc::new(demo::snapshot(61)));
    assert!(!h.state().procs.is_folded(root));
    // 新しく見えた物は畳む
    let mut next = demo::snapshot(62);
    let mut born = next.rows.iter().find(|r| r.id == root).unwrap().clone();
    born.id = procsight_core::ProcId { pid: 31_337, create_time: 1 };
    next.rows.push(born.clone());
    h.state_mut().set_snapshot(Arc::new(next));
    assert!(h.state().procs.is_folded(born.id));
    // 一覧から消えたら畳みの印も消す
    h.state_mut().set_snapshot(Arc::new(demo::snapshot(63)));
    assert!(!h.state().procs.auto_collapsed.contains(&born.id));

    // 既定(開く)のまま見えた物は、あとで「畳む」にしても畳み直さない。その後の新入りは畳む
    let mut h = drawn(demo::state());
    assert!(h.state().procs.collapsed.is_empty() && h.state().procs.auto_collapsed.is_empty());
    h.state_mut().cfg.tree_start = TreeStart::Collapsed;
    h.run_steps(1);
    let mut next = demo::snapshot(61);
    let mut born2 = next.rows.iter().find(|r| r.name == "dwm.exe").unwrap().clone();
    born2.id = procsight_core::ProcId { pid: 31_338, create_time: 1 };
    next.rows.push(born2.clone());
    h.state_mut().set_snapshot(Arc::new(next));
    let p = &h.state().procs;
    assert!(!p.is_folded(root) && p.is_folded(born2.id));
    // 「開く」に戻した後の新入りは開いたまま
    h.state_mut().cfg.tree_start = TreeStart::Expanded;
    h.run_steps(1);
    let mut next = demo::snapshot(62);
    let mut born3 = born2.clone();
    born3.id = procsight_core::ProcId { pid: 31_339, create_time: 1 };
    next.rows.push(born3.clone());
    h.state_mut().set_snapshot(Arc::new(next));
    assert!(!h.state().procs.is_folded(born3.id));
}

/// 「畳む」で畳んだ親の下でも、絞り込みに一致した行の数を見る(tree.rs の動きの確かめ。報告用に数を出す)
#[test]
fn tree_start_collapsed_with_filter() {
    let count = |tree_start: TreeStart| {
        let mut st = state_with(|c| c.tree_start = tree_start);
        st.procs.filter = "testee".into();
        let h = drawn(st);
        h.state().procs.visible.iter().filter(|v| !v.dimmed).count()
    };
    let (open, folded) = (count(TreeStart::Expanded), count(TreeStart::Collapsed));
    println!("絞り込み testee の一致: 開く {open} 行、畳む {folded} 行");
    assert_eq!(open, 13);
    assert_eq!(folded, 13, "絞り込みの間は「全部畳む」で畳んだ物を見ない");
    // 手で畳んだ物は v0.3.0 と同じに、絞り込みの間も畳んだまま(畳んだ親の下の一致は隠れる)
    let mut st = state_with(|c| c.tree_start = TreeStart::Expanded);
    let root = st.snap.find_pid(10_000).unwrap().id;
    st.procs.set_folded(root, true);
    st.procs.filter = "testee".into();
    let h = drawn(st);
    assert!(h.state().procs.visible.iter().filter(|v| !v.dimmed).count() < 13);
    assert!(h.state().procs.is_folded(root));
}

#[test]
fn s1_tree_start_collapsed_looks() {
    shoot("s1-tree-collapsed-start", BIG, || state_with(|c| c.tree_start = TreeStart::Collapsed));
}

/// グラフの長さ: CPU の推移・詳細のグラフの輪が設定の長さまで伸びる。減らすと古い方を捨てる
#[test]
fn graph_points_resize_history_rings() {
    // cpu_history_state(120 回分)の後で 300 にし、さらに 240 回流す
    let long = || {
        let mut st = cpu_history_state();
        st.cfg.graph_points = 300;
        st.procs.follow_config(&st.cfg);
        for seq in 121..=360u64 {
            let mut s = demo::snapshot(seq);
            let t = seq as f32;
            for r in s.rows.iter_mut().filter(|r| r.id.pid != 0) {
                if let Some(c) = r.cpu_pct.as_mut() {
                    *c = (*c + 20.0 * (t * 0.05 + r.id.pid as f32).sin()).max(0.0);
                }
            }
            st.set_snapshot(Arc::new(s));
        }
        st
    };
    let mut st = long();
    let id = target(&st, "browser.exe").id;
    assert_eq!(st.procs.cpu_history(id).unwrap().len(), 300);
    assert_eq!(st.procs.detail.cpu.capacity(), 300);
    assert_eq!(st.procs.detail.cpu.len(), 240, "選んだ後の 240 回");
    let tail: Vec<f32> = st.procs.cpu_history(id).unwrap().skip(240).collect();
    st.cfg.graph_points = 60;
    st.procs.follow_config(&st.cfg);
    assert_eq!(st.procs.cpu_history(id).unwrap().collect::<Vec<_>>(), tail, "新しい 60 回を残す");
    assert_eq!(st.procs.detail.mem.len(), 60);
    // 描く度に設定へ合わせる(設定の画面は値を変えるだけ)
    let mut h = drawn(long());
    h.state_mut().cfg.graph_points = 120;
    h.run_steps(1);
    assert_eq!(h.state().procs.cpu_history(id).unwrap().len(), 120);
    shoot("s1-graph-300", BIG, long);
}
