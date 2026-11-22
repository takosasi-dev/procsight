//! 前回の Snapshot と今回の RawSnapshot の差から、CPU 使用率・I/O の速さ・新しい行・終わった行を作る(FR-1・FR-7・D-9)。
//! 状態の列(FR-6・D-32)と親の決め方(FR-2・D-8)もここで決める。

use crate::model::*;
use std::collections::{HashMap, HashSet};

/// ThreadState の「待ち」(KTHREAD_STATE の Waiting)。§8.1 の probe で確かめる
pub const THREAD_STATE_WAITING: u32 = 5;
/// WaitReason の「一時停止」(KWAIT_REASON の Suspended)。§8.1 の probe で確かめる(事実20)
pub const WAIT_REASON_SUSPENDED: u32 = 5;
/// 新しい行・終わった行の色を残す回数(FR-7)
pub const HIGHLIGHT_UPDATES: u8 = 2;

pub fn is_suspended(threads: &[RawThread]) -> bool {
    !threads.is_empty()
        && threads
            .iter()
            .all(|t| t.state == THREAD_STATE_WAITING && t.wait_reason == WAIT_REASON_SUSPENDED)
}

fn cpu_time(p: &RawProcess) -> Option<u64> {
    Some(p.kernel_time?.saturating_add(p.user_time?))
}

/// 親を決める。親の PID のプロセスが無い・親の開始時刻が子より後(使い回し)・自分自身なら None
pub fn resolve_parent(p: &RawProcess, by_pid: &HashMap<u32, ProcId>) -> Option<ProcId> {
    if p.parent_pid == p.id.pid || p.id.pid == 0 {
        return None;
    }
    let parent = *by_pid.get(&p.parent_pid)?;
    (parent.create_time <= p.id.create_time).then_some(parent)
}

pub fn diff(
    prev: Option<&Snapshot>,
    raw: RawSnapshot,
    gpu: &GpuByPid,
    windows: &WindowInfo,
    touch: &crate::rules::TouchFreeSet,
    statics: &StaticCache,
) -> Snapshot {
    let touch_ids = touch.resolve(&raw);
    let by_pid: HashMap<u32, ProcId> = raw.processes.iter().map(|p| (p.id.pid, p.id)).collect();
    let prev_rows: HashMap<ProcId, &ProcessRow> = prev
        .map(|s| s.rows.iter().filter(|r| r.is_gone == 0).map(|r| (r.id, r)).collect())
        .unwrap_or_default();
    let elapsed_ms = prev.map(|s| raw.taken_at_ms.saturating_sub(s.taken_at_ms));
    let cpus = raw.logical_cpus.max(1) as f64;

    let mut rows = Vec::with_capacity(raw.processes.len() + 8);
    let mut totals = Totals::default();
    let mut seen = HashSet::with_capacity(raw.processes.len());

    for p in &raw.processes {
        seen.insert(p.id);
        let before = prev_rows.get(&p.id).copied();
        let now_cpu = cpu_time(p);
        let rate = |now: Option<u64>, then: Option<u64>| -> Option<f64> {
            let ms = elapsed_ms.filter(|&ms| ms > 0)?;
            Some(now?.saturating_sub(then?) as f64 * 1000.0 / ms as f64)
        };
        let cpu_pct = before.and_then(|b| {
            let ms = elapsed_ms.filter(|&ms| ms > 0)?;
            let used = now_cpu?.saturating_sub(b.cpu_time?) as f64; // 100 ナノ秒
            Some((used / (ms as f64 * 10_000.0) / cpus * 100.0).clamp(0.0, 100.0) as f32)
        });
        let touch_free = touch_ids.contains(&p.id);
        let state = if touch_free {
            ProcState::TouchFree
        } else if p.threads.is_empty() && p.id.pid != 0 {
            ProcState::Exiting
        } else if is_suspended(&p.threads) {
            ProcState::Suspended
        } else if windows.hung.contains(&p.id.pid) {
            ProcState::NotResponding
        } else {
            ProcState::Running
        };
        let name = if p.id.pid == 0 && p.image_name.is_empty() {
            "System Idle Process".to_string()
        } else {
            p.image_name.clone()
        };
        let is_new = match (prev, before) {
            (None, _) => 0,
            (Some(_), None) => HIGHLIGHT_UPDATES,
            (Some(_), Some(b)) => b.is_new.saturating_sub(1),
        };
        totals.processes += 1;
        totals.threads += p.threads.len() as u32;
        totals.handles += p.handle_count as u64;
        rows.push(ProcessRow {
            id: p.id,
            parent: resolve_parent(p, &by_pid),
            parent_pid: p.parent_pid,
            name,
            state,
            cpu_pct,
            mem: Some(p.private_working_set.unwrap_or(p.private_bytes)),
            working_set: p.working_set,
            commit: p.private_bytes,
            io_read_bps: before.and_then(|b| rate(p.io_read_bytes, b.io_read_total)),
            io_write_bps: before.and_then(|b| rate(p.io_write_bytes, b.io_write_total)),
            gpu_pct: gpu.get(&p.id.pid).copied(),
            threads: p.threads.len() as u32,
            handles: p.handle_count,
            base_priority: p.base_priority,
            session_id: p.session_id,
            touch_free,
            has_window: windows.visible.contains(&p.id.pid),
            static_info: if touch_free { None } else { statics.get(&p.id).cloned() },
            is_new,
            is_gone: 0,
            cpu_time: now_cpu,
            io_read_total: p.io_read_bytes,
            io_write_total: p.io_write_bytes,
        });
    }

    // 終わった行を 2 回の更新の間だけ残す(FR-7)
    if let Some(prev) = prev {
        for r in &prev.rows {
            if seen.contains(&r.id) {
                continue;
            }
            let left = if r.is_gone == 0 { HIGHLIGHT_UPDATES } else { r.is_gone - 1 };
            if left == 0 {
                continue;
            }
            let mut gone = r.clone();
            gone.is_gone = left;
            gone.is_new = 0;
            gone.cpu_pct = None;
            gone.io_read_bps = None;
            gone.io_write_bps = None;
            gone.gpu_pct = None;
            rows.push(gone);
        }
    }

    Snapshot {
        seq: prev.map_or(1, |s| s.seq + 1),
        taken_at_ms: raw.taken_at_ms,
        logical_cpus: raw.logical_cpus,
        rows,
        totals,
        perf: PerfSample::default(),
        touch_free: touch_ids,
        rows_stale: false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::rules::TouchFreeSet;

    pub fn proc_(pid: u32, ct: u64, ppid: u32, name: &str) -> RawProcess {
        RawProcess {
            id: ProcId { pid, create_time: ct },
            parent_pid: ppid,
            image_name: name.into(),
            threads: vec![RawThread { tid: pid + 1, state: 2, ..Default::default() }],
            kernel_time: Some(0),
            user_time: Some(0),
            private_working_set: Some(1024),
            private_bytes: 2048,
            io_read_bytes: Some(0),
            io_write_bytes: Some(0),
            base_priority: 8,
            ..Default::default()
        }
    }

    fn raw(at: u64, cpus: u32, ps: Vec<RawProcess>) -> RawSnapshot {
        RawSnapshot { taken_at_ms: at, logical_cpus: cpus, processes: ps }
    }

    fn none() -> TouchFreeSet {
        TouchFreeSet { names: vec![], include_descendants: true, ..Default::default() }
    }

    fn run(prev: Option<&Snapshot>, r: RawSnapshot) -> Snapshot {
        diff(prev, r, &GpuByPid::new(), &WindowInfo::default(), &none(), &StaticCache::new())
    }

    #[test]
    fn first_snapshot_has_no_rates() {
        let s = run(None, raw(1000, 4, vec![proc_(10, 1, 0, "a.exe")]));
        assert_eq!(s.rows[0].cpu_pct, None);
        assert_eq!(s.rows[0].io_read_bps, None);
        assert_eq!(s.rows[0].is_new, 0);
        assert_eq!(s.seq, 1);
    }

    #[test]
    fn cpu_pct_divides_by_logical_cpus() {
        let s1 = run(None, raw(1000, 4, vec![proc_(10, 1, 0, "a.exe")]));
        let mut p = proc_(10, 1, 0, "a.exe");
        // 1 秒で 1 コア分(1000 ms = 10,000,000 単位)を使った → 4 コアで 25%
        p.user_time = Some(10_000_000);
        let s2 = run(Some(&s1), raw(2000, 4, vec![p]));
        let cpu = s2.rows[0].cpu_pct.unwrap();
        assert!((cpu - 25.0).abs() < 0.01, "{cpu}");
    }

    #[test]
    fn cpu_pct_is_clamped() {
        let s1 = run(None, raw(1000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        let mut p = proc_(10, 1, 0, "a.exe");
        p.kernel_time = Some(50_000_000);
        let s2 = run(Some(&s1), raw(2000, 1, vec![p]));
        assert_eq!(s2.rows[0].cpu_pct, Some(100.0));
    }

    #[test]
    fn io_rate_bytes_per_second() {
        let s1 = run(None, raw(1000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        let mut p = proc_(10, 1, 0, "a.exe");
        p.io_write_bytes = Some(1_000_000);
        let s2 = run(Some(&s1), raw(1500, 1, vec![p]));
        assert_eq!(s2.rows[0].io_write_bps, Some(2_000_000.0));
    }

    #[test]
    fn missing_reserved_fields_give_dash() {
        let mut p = proc_(10, 1, 0, "a.exe");
        p.kernel_time = None;
        p.private_working_set = None;
        let s1 = run(None, raw(1000, 1, vec![p.clone()]));
        let s2 = run(Some(&s1), raw(2000, 1, vec![p]));
        assert_eq!(s2.rows[0].cpu_pct, None);
        assert_eq!(s2.rows[0].mem, Some(2048), "コミットに戻す(D-10)");
    }

    #[test]
    fn new_and_gone_rows_last_two_updates() {
        let s1 = run(None, raw(1000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        let s2 = run(Some(&s1), raw(2000, 1, vec![proc_(10, 1, 0, "a.exe"), proc_(20, 5, 10, "b.exe")]));
        assert_eq!(s2.rows.iter().find(|r| r.id.pid == 20).unwrap().is_new, 2);
        let s3 = run(Some(&s2), raw(3000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        let gone = s3.rows.iter().find(|r| r.id.pid == 20).unwrap();
        assert_eq!(gone.is_gone, 2);
        assert_eq!(s3.totals.processes, 1);
        let s4 = run(Some(&s3), raw(4000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        assert_eq!(s4.rows.iter().find(|r| r.id.pid == 20).unwrap().is_gone, 1);
        let s5 = run(Some(&s4), raw(5000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        assert!(s5.rows.iter().all(|r| r.id.pid != 20));
        // 新しい行は 2 → 1 → 0
        let t1 = run(Some(&s1), raw(2000, 1, vec![proc_(30, 9, 0, "c.exe")]));
        let t2 = run(Some(&t1), raw(3000, 1, vec![proc_(30, 9, 0, "c.exe")]));
        let t3 = run(Some(&t2), raw(4000, 1, vec![proc_(30, 9, 0, "c.exe")]));
        assert_eq!(t2.rows.iter().find(|r| r.id.pid == 30).unwrap().is_new, 1);
        assert_eq!(t3.rows.iter().find(|r| r.id.pid == 30).unwrap().is_new, 0);
    }

    #[test]
    fn pid_reuse_is_a_new_process() {
        let s1 = run(None, raw(1000, 1, vec![proc_(10, 1, 0, "a.exe")]));
        let mut p = proc_(10, 99, 0, "other.exe");
        p.user_time = Some(10_000_000);
        let s2 = run(Some(&s1), raw(2000, 1, vec![p]));
        let row = s2.rows.iter().find(|r| r.id.create_time == 99).unwrap();
        assert_eq!(row.cpu_pct, None, "使い回しの PID の差は取らない");
        assert_eq!(row.is_new, 2);
        assert!(s2.rows.iter().any(|r| r.id.create_time == 1 && r.is_gone == 2));
    }

    #[test]
    fn idle_process_named() {
        let s = run(None, raw(1000, 1, vec![proc_(0, 0, 0, "")]));
        assert_eq!(s.rows[0].name, "System Idle Process");
        assert_eq!(s.rows[0].parent, None);
    }

    #[test]
    fn states_in_priority_order() {
        let mut suspended = proc_(10, 1, 0, "s.exe");
        suspended.threads = vec![
            RawThread { state: 5, wait_reason: 5, ..Default::default() },
            RawThread { state: 5, wait_reason: 5, ..Default::default() },
        ];
        let mut partly = proc_(11, 1, 0, "p.exe");
        partly.threads = vec![
            RawThread { state: 5, wait_reason: 5, ..Default::default() },
            RawThread { state: 2, ..Default::default() },
        ];
        let mut exiting = proc_(12, 1, 0, "e.exe");
        exiting.threads.clear();
        let hung = proc_(13, 1, 0, "h.exe");
        let touch = proc_(14, 1, 0, "Game.EXE");
        let mut win = WindowInfo::default();
        win.hung.insert(13);
        win.hung.insert(14);
        win.visible.insert(13);
        let tf = TouchFreeSet { names: vec!["game.exe".into()], include_descendants: true, ..Default::default() };
        let s = diff(
            None,
            raw(1, 1, vec![suspended, partly, exiting, hung, touch]),
            &GpuByPid::new(),
            &win,
            &tf,
            &StaticCache::new(),
        );
        let st = |pid| s.rows.iter().find(|r| r.id.pid == pid).unwrap().state;
        assert_eq!(st(10), ProcState::Suspended);
        assert_eq!(st(11), ProcState::Running);
        assert_eq!(st(12), ProcState::Exiting);
        assert_eq!(st(13), ProcState::NotResponding);
        assert_eq!(st(14), ProcState::TouchFree);
        assert!(s.rows.iter().find(|r| r.id.pid == 13).unwrap().has_window);
    }

    #[test]
    fn touch_free_rows_never_carry_static_info() {
        let mut statics = StaticCache::new();
        let id = ProcId { pid: 14, create_time: 1 };
        statics.insert(id, std::sync::Arc::new(StaticInfo { path: Some("x".into()), ..Default::default() }));
        let tf = TouchFreeSet { names: vec!["game.exe".into()], include_descendants: true, ..Default::default() };
        let s = diff(
            None,
            raw(1, 1, vec![proc_(14, 1, 0, "game.exe")]),
            &GpuByPid::new(),
            &WindowInfo::default(),
            &tf,
            &statics,
        );
        assert!(s.rows[0].static_info.is_none());
        assert!(s.touch_free.contains(&id));
    }

    #[test]
    fn parent_must_be_older() {
        let s = run(
            None,
            raw(1, 1, vec![proc_(10, 100, 0, "p.exe"), proc_(20, 200, 10, "child.exe"), proc_(30, 50, 10, "orphan.exe")]),
        );
        let parent = |pid| s.rows.iter().find(|r| r.id.pid == pid).unwrap().parent;
        assert_eq!(parent(20), Some(ProcId { pid: 10, create_time: 100 }));
        assert_eq!(parent(30), None, "親の開始時刻が子より後なら使い回し(D-8)");
    }
}
