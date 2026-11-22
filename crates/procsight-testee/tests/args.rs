//! testee の引数の煙テスト(窓を出さない物): --children/--depth・--alloc・--hold-file。
//! 子は Drop では終わらないので、SPI で子孫を集めて葉から ops::terminate で終わらせ、残りが 0 なのを確かめる。

mod common;

use common::{Testee, find, wait_for};
use procsight_core::{ProcId, RawProcess};
use procsight_win::nt::spi::{Fields, SpiReader};
use procsight_win::{ProcessGate, ops, static_info};
use std::collections::HashSet;
use std::time::Duration;

/// root とその子孫(親の開始時刻で使い回しを除く)。深い物が先
fn tree(root: ProcId) -> Vec<ProcId> {
    let ps: Vec<RawProcess> = SpiReader::new().read(Fields::NONE).unwrap();
    let mut out = vec![(root, 0usize)];
    let mut i = 0;
    while i < out.len() {
        let (parent, depth) = out[i];
        for p in ps.iter().filter(|p| p.parent_pid == parent.pid && p.id.create_time >= parent.create_time && p.id != parent) {
            out.push((p.id, depth + 1));
        }
        i += 1;
    }
    out.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    out.into_iter().map(|(id, _)| id).collect()
}

#[test]
fn children_make_thirteen_and_all_end() {
    let t = Testee::start(&["--children", "3", "--depth", "2"]);
    let ids = wait_for(Duration::from_secs(15), || {
        let ids = tree(t.id);
        (ids.len() == 13).then_some(ids)
    });
    let g = ProcessGate::new(std::process::id(), static_info::self_elevated());
    // 13 にならなくても、見つかった物は全部終わらせてから判定する
    let ids = ids.unwrap_or_else(|| tree(t.id));
    for id in &ids {
        let _ = ops::terminate(&g, *id);
    }
    let set: HashSet<ProcId> = ids.iter().copied().collect();
    let gone = wait_for(Duration::from_secs(5), || {
        let ps = SpiReader::new().read(Fields::NONE).ok()?;
        ps.iter().all(|p| !set.contains(&p.id)).then_some(())
    });
    assert_eq!(ids.len(), 13, "--children 3 --depth 2 は 1 + 3 + 9");
    assert!(gone.is_some(), "全部終わる");
}

#[test]
fn alloc_raises_private_working_set() {
    let t = Testee::start(&["--alloc", "50"]);
    let ws = wait_for(Duration::from_secs(10), || {
        let ws = find(t.id.pid)?.private_working_set?;
        (ws >= 50 << 20).then_some(ws)
    });
    assert!(ws.is_some(), "プライベート作業セットが 50 MB 以上");
}

#[test]
fn hold_file_denies_sharing() {
    let path = std::env::temp_dir().join(format!("psight-hold-{}.txt", std::process::id()));
    let p = path.to_string_lossy().into_owned();
    let t = Testee::start(&["--hold-file", &p]);
    let denied = wait_for(Duration::from_secs(5), || match std::fs::File::open(&path) {
        Err(e) if e.raw_os_error() == Some(32) => Some(()), // ERROR_SHARING_VIOLATION
        _ => None,
    });
    drop(t);
    let _ = wait_for(Duration::from_secs(2), || std::fs::remove_file(&path).ok());
    assert!(denied.is_some(), "共有なしで開いたまま持つ");
}
