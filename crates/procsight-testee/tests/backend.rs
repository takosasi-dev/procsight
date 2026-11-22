//! Backend の結合テスト。Collector の Snapshot・StaticInfo・Worker の Job と、触れないプロセスを関門が開かないこと(AC-30 の一部)。

mod common;

use common::{Testee, wait_for};
use procsight_core::rules::{Refusal, TouchFreeSet};
use procsight_core::{Priority, ProcState};
use procsight_win::gate::GateError;
use procsight_win::jobs::{Job, JobResult, OpError, Target};
use procsight_win::{Backend, BackendConfig};
use std::sync::Arc;
use std::time::Duration;

/// テストを回す PC で動いている他のプロセス(ゲームを含む)を StaticInfo のために開かないよう、testee とこのテストの exe 以外の
/// 今動いている名前を全部「触れない」にする。子孫は含めない(cargo.exe の子孫にこのテストと testee がいるため)
fn start(touch: &[&str]) -> Backend {
    let me = std::env::current_exe().unwrap();
    let me = me.file_name().unwrap().to_string_lossy().to_lowercase();
    let mut names: Vec<String> = touch.iter().map(|s| s.to_string()).collect();
    for p in procsight_win::nt::spi::query_processes().unwrap() {
        let n = p.image_name.to_lowercase();
        if !n.is_empty() && n != me && n != "procsight-testee.exe" && !names.contains(&n) {
            names.push(n);
        }
    }
    let cfg = BackendConfig {
        interval_ms: 300,
        touch_free: TouchFreeSet { names, include_descendants: false, ..Default::default() },
    };
    Backend::start(cfg, Arc::new(|| {}))
}

fn recv(b: &Backend, n: usize) -> Vec<JobResult> {
    let mut out = Vec::new();
    wait_for(Duration::from_secs(30), || {
        while let Some(r) = b.try_recv() {
            out.push(r);
        }
        (out.len() >= n).then_some(())
    })
    .unwrap_or_else(|| panic!("結果が {n} 個そろわない({} 個)", out.len()));
    out
}

#[test]
fn touch_free_testee_is_never_opened() {
    let b = start(&["procsight-testee.exe"]);
    let mut t = Testee::start(&["--threads", "1"]);
    let s = wait_for(Duration::from_secs(10), || {
        let s = b.snapshot();
        s.find(t.id).filter(|r| r.state == ProcState::TouchFree).map(|_| s.clone())
    })
    .expect("testee が「触れない」になる");
    assert!(s.touch_free.contains(&t.id));
    let target = Target { id: t.id, name: "procsight-testee.exe".into() };
    let exe = env!("CARGO_BIN_EXE_procsight-testee").to_string();
    let jobs = vec![
        Job::General { id: t.id },
        Job::Modules { id: t.id },
        Job::Handles { id: t.id },
        Job::Terminate { targets: vec![target.clone()], tree: false },
        Job::SetPriority { target: target.clone(), priority: Priority::High },
        Job::GetAffinity { target: target.clone() },
        Job::SetAffinity { target: target.clone(), mask: 1 },
        Job::SetEfficiency { target: target.clone(), on: true },
        Job::Suspend { target: target.clone() },
        Job::Resume { target: target.clone() },
        Job::ResumeMany { targets: vec![target.clone()] },
        Job::BringToFront { target: target.clone() },
        Job::Icon { path: exe.clone() },
        Job::Signature { path: exe },
        Job::FindLocking { paths: vec!["C:\\x.txt".into()] },
    ];
    let n = jobs.len();
    for j in jobs {
        b.submit(j);
    }
    let results = recv(&b, n);
    let tf = OpError::Gate(GateError::TouchFree);
    for r in &results {
        match r {
            JobResult::Op { result, .. } => assert_eq!(result.as_ref().err(), Some(&tf), "{r:?}"),
            JobResult::Terminated { results, .. } => {
                assert!(matches!(results[0].1, Err(OpError::Refused(Refusal::TouchFree)) | Err(OpError::Gate(GateError::TouchFree))))
            }
            JobResult::General { info, .. } => assert_eq!(info.as_ref().err(), Some(&tf)),
            JobResult::Modules { result, .. } => assert_eq!(result.as_ref().err(), Some(&tf)),
            JobResult::Handles { result, .. } => assert!(result.is_err()),
            JobResult::Affinity { result, .. } => assert!(result.is_err()),
            JobResult::Icon { rgba, .. } => assert!(rgba.is_none()),
            JobResult::Locking { result, .. } => assert!(matches!(result, Err(OpError::Blocked(_)))),
            _ => {}
        }
    }
    // しばらく Collector を回しても、testee の PID を 1 回も開いていない
    std::thread::sleep(Duration::from_secs(1));
    let d = b.diagnostics();
    assert!(d.open_counts.iter().all(|(pid, _)| *pid != t.id.pid), "{:?}", d.open_counts);
    assert!(b.snapshot().find(t.id).unwrap().static_info.is_none());
    assert!(t.alive());
    let (tf_opens, total) = b.shutdown();
    assert_eq!(tf_opens, 0);
    assert!(total > 0, "他のプロセスの StaticInfo は開いて取る");
}

#[test]
fn snapshot_static_info_and_operations() {
    let b = start(&[]);
    let mut t = Testee::start(&["--busy", "1"]);
    let s = wait_for(Duration::from_secs(10), || {
        let s = b.snapshot();
        let r = s.find(t.id)?;
        (r.cpu_pct.is_some() && r.static_info.is_some() && s.perf.cpu_total.is_some()).then(|| s.clone())
    })
    .expect("CPU・StaticInfo・全体の CPU が出る");
    let row = s.find(t.id).unwrap();
    assert!(row.cpu_pct.unwrap() > 0.0);
    assert!(row.path().unwrap().to_lowercase().ends_with("procsight-testee.exe"));
    assert_eq!(row.state, ProcState::Running);
    assert!(s.perf.mem.is_some() && s.totals.processes > 10);
    let d = b.diagnostics();
    assert!(d.api.spi_times && d.api.spi_io && d.api.spi_private_ws && d.api.suspend && d.api.pdh_cpu, "{:?}", d.api);

    let target = Target { id: t.id, name: row.name.clone() };
    b.submit(Job::Suspend { target: target.clone() });
    assert!(matches!(&recv(&b, 1)[0], JobResult::Op { result: Ok(()), .. }));
    wait_for(Duration::from_secs(5), || (b.snapshot().find(t.id)?.state == ProcState::Suspended).then_some(()))
        .expect("一時停止中になる");
    b.submit(Job::Resume { target: target.clone() });
    assert!(matches!(&recv(&b, 1)[0], JobResult::Op { result: Ok(()), .. }));

    // 自分自身は終了を断る(INV-4)
    let me = b.snapshot().find_pid(std::process::id()).map(|r| Target { id: r.id, name: r.name.clone() }).unwrap();
    b.submit(Job::Terminate { targets: vec![me, target], tree: false });
    match &recv(&b, 1)[0] {
        JobResult::Terminated { results, .. } => {
            assert_eq!(results[0].1, Err(OpError::Refused(Refusal::SelfProcess)));
            assert_eq!(results[1].1, Ok(()));
        }
        r => panic!("{r:?}"),
    }
    assert_eq!(t.child.wait().unwrap().code(), Some(1));
    b.shutdown();
}
