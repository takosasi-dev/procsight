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
