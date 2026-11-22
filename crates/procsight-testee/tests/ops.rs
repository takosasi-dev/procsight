//! ops の結合テスト。自分で起動した testee にだけ操作する(終了・優先度・割り当て・効率モード・一時停止・再開・入れ替わり)。

mod common;

use common::{Testee, cpu_time, find, wait_for};
use procsight_core::diff::is_suspended;
use procsight_core::{Priority, ProcId};
use procsight_win::gate::GateError;
use procsight_win::jobs::OpError;
use procsight_win::{ProcessGate, ops, static_info};
use std::time::Duration;
use windows::Win32::System::Threading::GetPriorityClass;

fn gate() -> ProcessGate {
    ProcessGate::new(std::process::id(), static_info::self_elevated())
}

/// GetPriorityClass で読み返す(関門で開く)
fn priority_class(g: &ProcessGate, id: ProcId) -> u32 {
    let h = g.open(id, Default::default()).expect("開ける");
    // SAFETY: h は関門で開いた PROCESS_QUERY_LIMITED_INFORMATION のハンドル
    unsafe { GetPriorityClass(h.raw()) }
}

#[test]
fn priority_affinity_efficiency() {
    let t = Testee::start(&["--threads", "1"]);
    let g = gate();
    let expect = [
        (Priority::Idle, 0x40, 4),
        (Priority::BelowNormal, 0x4000, 6),
        (Priority::Normal, 0x20, 8),
        (Priority::AboveNormal, 0x8000, 10),
        (Priority::High, 0x80, 13),
    ];
    for (p, class, base) in expect {
        ops::set_priority(&g, t.id, p).unwrap();
        assert_eq!(priority_class(&g, t.id), class, "{p:?}");
        assert_eq!(find(t.id.pid).unwrap().base_priority, base, "SPI の BasePriority(FR-12)");
    }

    let (mask, system) = ops::get_affinity(&g, t.id).unwrap();
    assert_eq!(mask, system);
    ops::set_affinity(&g, t.id, 1).unwrap();
    assert_eq!(ops::get_affinity(&g, t.id).unwrap().0, 1);
    assert!(matches!(ops::set_affinity(&g, t.id, 0), Err(OpError::Blocked(_))));
    ops::set_affinity(&g, t.id, system).unwrap();

    ops::set_efficiency(&g, t.id, true).unwrap();
    assert_eq!(priority_class(&g, t.id), 0x40);
    assert_eq!(ops::efficiency(&g, t.id), Some(true));
    ops::set_efficiency(&g, t.id, false).unwrap();
    assert_eq!(priority_class(&g, t.id), 0x20);
    assert_eq!(ops::efficiency(&g, t.id), Some(false));
}

#[test]
fn suspend_and_resume() {
    let t = Testee::start(&["--busy", "1", "--threads", "2"]);
    let g = gate();
    ops::suspend(&g, t.id).unwrap();
    // 一時停止はすぐには全スレッドに届かないことがあるので、待って確かめる
    wait_for(Duration::from_secs(2), || is_suspended(&find(t.id.pid)?.threads).then_some(()))
        .expect("全スレッドが待ち+一時停止の理由になる");
    let c0 = cpu_time(t.id.pid);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(cpu_time(t.id.pid), c0, "止めている間は CPU 時間が増えない");
    ops::resume(&g, t.id).unwrap();
    assert!(!is_suspended(&find(t.id.pid).unwrap().threads));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(cpu_time(t.id.pid) > c0 + 5_000_000, "再開したら CPU を使う");
}

#[test]
fn terminate_and_replaced() {
    let mut t = Testee::start(&["--threads", "1"]);
    let g = gate();
    let wrong = ProcId { pid: t.id.pid, create_time: t.id.create_time + 1 };
    assert_eq!(ops::terminate(&g, wrong), Err(OpError::Gate(GateError::Replaced)), "開始時刻が違えば何もしない(D-8)");
    assert!(t.alive());
    ops::terminate(&g, t.id).unwrap();
    assert_eq!(t.child.wait().unwrap().code(), Some(1), "TerminateProcess(h, 1)");
}

#[test]
fn touch_free_is_never_opened() {
    let mut t = Testee::start(&["--threads", "1"]);
    let g = gate();
    g.set_touch_free([t.id].into());
    assert_eq!(ops::terminate(&g, t.id), Err(OpError::Gate(GateError::TouchFree)));
    assert_eq!(ops::suspend(&g, t.id), Err(OpError::Gate(GateError::TouchFree)));
    assert_eq!(ops::set_priority(&g, t.id, Priority::High), Err(OpError::Gate(GateError::TouchFree)));
    let target = procsight_win::jobs::Target { id: t.id, name: "procsight-testee.exe".into() };
    assert_eq!(ops::bring_to_front(&g, &target), Err(OpError::Gate(GateError::TouchFree)));
    assert!(g.open_counts().is_empty());
    assert!(t.alive());
}
