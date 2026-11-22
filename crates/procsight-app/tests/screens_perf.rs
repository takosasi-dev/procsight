//! S2 パフォーマンスのタブ(担当 B)を、ダーク・ライト・崩れやすい条件で PNG に描いて確かめる。
//! 上位のプロセスの行を押すと、プロセスのタブのその行へ移ることも確かめる。
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

fn on_page(page: Page, per_core: bool) -> State {
    let mut st = with_tab(Tab::Performance);
    st.perf.page = page;
    st.perf.per_core = per_core;
    st
}

#[test]
fn s2_performance() {
    for (name, page, per_core) in [
        ("s2-cpu", Page::Cpu, false),
        ("s2-cpu-cores", Page::Cpu, true),
        ("s2-memory", Page::Memory, false),
        ("s2-disk", Page::Disk("0 C:".into()), false),
        ("s2-disk-idle", Page::Disk("1 D:".into()), false),
        ("s2-net", Page::Net("Wi-Fi".into()), false),
        ("s2-gpu", Page::Gpu("luid_0x00000000_0x0000D1A3".into()), false),
    ] {
        shoot(name, BIG, || on_page(page.clone(), per_core));
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
fn s2_stress() {
    shoot_stress("s2-cpu", || on_page(Page::Cpu, false));
    shoot_stress("s2-cpu-cores", || on_page(Page::Cpu, true));
    shoot_stress("s2-memory", || on_page(Page::Memory, false));
    shoot_stress("s2-disk", || on_page(Page::Disk("0 C:".into()), false));
    shoot_stress("s2-net", || on_page(Page::Net("Wi-Fi".into()), false));
}

/// 実機に多い形: 論理プロセッサ 32 個、長い CPU・GPU・アダプターの名前、CPU の作りが取れない
fn big_machine() -> State {
    let mut st = with_tab(Tab::Performance);
    st.env.cpu_name = Some("Example Corporation Hyper Performance 9 Ultra 32-Thread Desktop Processor".into());
    st.env.cpu_info = None;
    for seq in 61..=120 {
        let mut s = demo::snapshot(seq);
        let base = s.perf.cpu_per_core.clone();
        s.perf.cpu_per_core = (0..32).map(|i| (base[i % base.len()] + i as f32 * 2.3) % 100.0).collect();
        for n in s.perf.nets.iter_mut().flatten() {
            n.name = "Example Wireless Network Adapter 6E AX411 160MHz".into();
        }
        for g in s.perf.gpus.iter_mut().flatten() {
            g.name = "Example Graphics GeForce Ultra 9090 Ti Laptop GPU".into();
        }
        st.set_snapshot(Arc::new(s));
    }
    st
}

#[test]
fn s2_big_machine() {
    let net = "Example Wireless Network Adapter 6E AX411 160MHz".to_string();
    shoot("s2-many-cores", BIG, || {
        let mut st = big_machine();
        st.perf.per_core = true;
        st
    });
    shoot("s2-long-names-cpu", BIG, big_machine);
    let n = net.clone();
    shoot("s2-long-names-net", BIG, move || {
        let mut st = big_machine();
        st.perf.page = Page::Net(n.clone());
        st
    });
    shoot_stress("s2-many-cores", || {
        let mut st = big_machine();
        st.perf.per_core = true;
        st
    });
    shoot_stress("s2-long-names-gpu", || {
        let mut st = big_machine();
        st.perf.page = Page::Gpu("luid_0x00000000_0x0000D1A3".into());
        st
    });
}

/// 上位のプロセスの行を押すと、プロセスのタブに移ってその行が選ばれる(触れないプロセスでも選ぶだけはできる)
#[test]
fn s2_top_process_jumps_to_processes_tab() {
    let st = on_page(Page::Cpu, false);
    let top = st.perf.tops.cpu.first().cloned().expect("CPU の上位が空");
    assert_eq!(top.name, "AceGame.exe");
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    h.get_by_label(&format!("{}(PID {})", top.name, top.id.pid)).click();
    h.run_steps(4);
    assert_eq!(h.state().tab, Tab::Processes);
    assert_eq!(h.state().procs.selected, vec![top.id]);
    assert_eq!(h.state().procs.scroll_to, None, "移った後は scroll_to を使い切る");

    // メモリのページでも同じ。2 番目の行を押す
    let mut st = on_page(Page::Memory, false);
    st.procs.selected.clear();
    let second = st.perf.tops.mem[1].clone();
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    h.get_by_label(&format!("{}(PID {})", second.name, second.id.pid)).click();
    h.run_steps(4);
    assert_eq!(h.state().tab, Tab::Processes);
    assert_eq!(h.state().procs.selected, vec![second.id]);
}

/// 下の段(数字の格子と上位のプロセス)まで見えるよう、縦に長い窓で狭い幅・文字 160% を描く
#[test]
fn s2_tall() {
    for (name, page) in [("cpu", Page::Cpu), ("memory", Page::Memory), ("disk", Page::Disk("0 C:".into())), ("net", Page::Net("Wi-Fi".into()))] {
        let mut h = harness(on_page(page.clone(), false), true, [760.0, 1300.0]);
        save(&mut h, &format!("s2-tall-{name}-narrow"));
        let mut st = on_page(page, false);
        st.cfg.font_scale = 1.6;
        let mut h = harness(st, true, [1280.0, 1600.0]);
        save(&mut h, &format!("s2-tall-{name}-scale160"));
    }
}

/// グラフを指すと、その時の値の札が出る
#[test]
fn s2_graph_hover() {
    let mut h = harness(on_page(Page::Disk("0 C:".into()), false), true, BIG);
    h.run_steps(2);
    h.hover_at(egui::pos2(1100.0, 300.0));
    // 札は少し待ってから出る(egui の tooltip_delay)
    h.run_steps(60);
    h.hover_at(egui::pos2(1101.0, 300.0));
    save(&mut h, "s2-hover-dark");
}

/// 更新間隔を変えた直後(履歴が空)も、取れないとは出さずに空のグラフを出す
#[test]
fn s2_after_interval_change() {
    for page in [Page::Disk("0 C:".into()), Page::Net("Wi-Fi".into()), Page::Gpu("luid_0x00000000_0x0000D1A3".into()), Page::Memory, Page::Cpu] {
        let mut st = on_page(page.clone(), false);
        st.perf.clear();
        let mut h = harness(st, true, BIG);
        h.run_steps(2);
        assert!(h.query_by_label_contains("取得できません").is_none(), "{page:?}");
    }
    let mut st = on_page(Page::Disk("0 C:".into()), false);
    st.perf.clear();
    let mut h = harness(st, true, BIG);
    save(&mut h, "s2-after-clear-dark");
}

/// ネットワークのページには上位のプロセスを出さない(プロセスごとの通信量は非スコープ)
#[test]
fn s2_net_page_has_no_process_table() {
    let st = on_page(Page::Net("Wi-Fi".into()), false);
    let any = st.perf.tops.cpu[0].clone();
    let mut h = harness(st, true, BIG);
    h.run_steps(2);
    assert!(h.query_by_label(&format!("{}(PID {})", any.name, any.id.pid)).is_none());
}
