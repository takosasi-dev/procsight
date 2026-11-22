//! 確かめ用の偽の Snapshot と JobResult(テストと、統合担当が README の画面写真を撮るのに使う)。
//! Backend を起こさずに、全部の画面の部品を描ける State を作る。名前とパスは架空(AceGame.exe など)。

use crate::app::{Load, State};
use crate::details::DTab;
use procsight_core::config::Config;
use procsight_core::startup::{Approved, StartupItem, StartupLocation};
use procsight_core::*;
use procsight_win::jobs::*;
use procsight_win::{ApiAvailability, Diagnostics, EnvInfo};
use std::sync::{Arc, Mutex};

pub const SELF_PID: u32 = 7500;
/// 2026-11-22 09:00:00 UTC あたりの FILETIME
const BASE_FT: u64 = (1_795_338_000u64 + 11_644_473_600) * 10_000_000;

pub fn env() -> EnvInfo {
    EnvInfo {
        self_pid: SELF_PID,
        elevated: false,
        system_root: "C:\\Windows".into(),
        program_files: "C:\\Program Files".into(),
        user: Some("PC\\user".into()),
        logical_cpus: 8,
        processor_groups: 1,
        utc_offset_minutes: 9 * 60,
        self_final_path: "C:\\Users\\user\\Desktop\\ProcSight.exe".into(),
        cpu_name: Some("Example CPU 8-Core Processor".into()),
    }
}

/// pid から決まる 0〜1 の値(偽のデータを毎回同じにする)
fn noise(pid: u32, k: u32) -> f32 {
    let x = pid.wrapping_mul(2_654_435_761).wrapping_add(k.wrapping_mul(40_503));
    (x >> 8) as f32 / (1u32 << 24) as f32
}

struct B {
    rows: Vec<ProcessRow>,
}

impl B {
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, pid: u32, parent: Option<u32>, parent_pid: u32, name: &str, path: Option<&str>, user: Option<&str>, cpu: f32, mem_mb: f32) -> &mut ProcessRow {
        let parent_id = parent.and_then(|p| self.rows.iter().find(|r| r.id.pid == p).map(|r| r.id));
        let ct = BASE_FT + pid as u64 * 10_000_000;
        let static_info = Some(Arc::new(StaticInfo {
            path: path.map(Into::into),
            user: user.map(Into::into),
            elevated: Some(false),
            critical: Some(false),
            arch: Some(Arch::X64),
            error: None,
        }));
        let mem = (mem_mb * 1024.0 * 1024.0) as u64;
        self.rows.push(ProcessRow {
            id: ProcId { pid, create_time: if pid == 0 { 0 } else { ct } },
            parent: parent_id,
            parent_pid,
            name: name.into(),
            state: ProcState::Running,
            cpu_pct: Some(cpu),
            mem: Some(mem),
            working_set: mem + mem / 3,
            commit: mem + mem / 5,
            io_read_bps: Some((noise(pid, 1) * 40_000.0).floor() as f64),
            io_write_bps: Some((noise(pid, 2) * 120_000.0).floor() as f64),
            gpu_pct: Some((noise(pid, 3) * 2.0 * 10.0).round() / 10.0),
            threads: 4 + (noise(pid, 4) * 60.0) as u32,
            handles: 80 + (noise(pid, 5) * 2000.0) as u32,
            base_priority: 8,
            session_id: 1,
            touch_free: false,
            has_window: false,
            static_info,
            is_new: 0,
            is_gone: 0,
            cpu_time: Some(pid as u64 * 1_234_567),
            io_read_total: None,
            io_write_total: None,
        });
        self.rows.last_mut().unwrap()
    }
}

const SYS32: &str = "C:\\Windows\\System32";

/// 見本の Snapshot。`seq` を変えると CPU などが少し動く
pub fn snapshot(seq: u64) -> Snapshot {
    let mut b = B { rows: Vec::new() };
    let s = |n: &str| format!("{SYS32}\\{n}");
    let sys = Some("NT AUTHORITY\\SYSTEM");
    let me = Some("PC\\user");
    b.add(0, None, 0, "System Idle Process", None, None, 81.6, 0.008).static_info = None;
    b.add(4, None, 0, "System", None, None, 0.4, 0.1).static_info = None;
    b.add(140, Some(4), 4, "Registry", None, None, 0.0, 41.0).static_info = None;
    b.add(480, Some(4), 4, "smss.exe", Some(&s("smss.exe")), sys, 0.0, 0.3);
    b.add(652, None, 560, "csrss.exe", None, None, 0.1, 2.1).static_info = None;
    b.add(760, None, 560, "wininit.exe", Some(&s("wininit.exe")), sys, 0.0, 1.2);
    b.add(900, Some(760), 760, "services.exe", Some(&s("services.exe")), sys, 0.1, 6.0);
    b.add(920, Some(760), 760, "lsass.exe", Some(&s("lsass.exe")), sys, 0.0, 9.8);
    for (i, k) in ["DcomLaunch", "RpcSs", "netsvcs", "LocalService", "NetworkService", "Audiosrv"].iter().enumerate() {
        let pid = 1000 + i as u32 * 40;
        let _ = k;
        b.add(pid, Some(900), 900, "svchost.exe", Some(&s("svchost.exe")), sys, noise(pid, 9) * 1.5, 8.0 + noise(pid, 7) * 40.0);
    }
    b.add(3100, Some(760), 760, "dwm.exe", Some(&s("dwm.exe")), Some("Window Manager\\DWM-1"), 1.8, 96.0);
    b.add(5120, None, 5000, "explorer.exe", Some("C:\\Windows\\explorer.exe"), me, 0.4, 112.3).has_window = true;
    let chrome = "C:\\Program Files\\Browser\\browser.exe";
    b.add(8812, Some(5120), 5120, "browser.exe", Some(chrome), me, 3.2, 410.0).has_window = true;
    for i in 0..3u32 {
        b.add(8900 + i * 4, Some(8812), 8812, "browser.exe", Some(chrome), me, noise(i, 1) * 2.0, 120.0 + i as f32 * 33.0);
    }
    {
        let g = b.add(9100, Some(5120), 5120, "AceGame.exe", None, None, 41.0, 3174.4);
        g.touch_free = true;
        g.state = ProcState::TouchFree;
        g.static_info = None;
        g.has_window = true;
        g.gpu_pct = Some(62.0);
    }
    {
        let g = b.add(9180, Some(9100), 9100, "AceGameHelper.exe", None, None, 1.2, 64.0);
        g.touch_free = true;
        g.state = ProcState::TouchFree;
        g.static_info = None;
    }
    b.add(7000, Some(5120), 5120, "notepad.exe", Some(&s("notepad.exe")), me, 0.0, 18.6).has_window = true;
    b.add(7300, Some(5120), 5120, "slowapp.exe", Some("C:\\Tools\\slowapp.exe"), me, 0.0, 44.0).state = ProcState::NotResponding;
    b.add(7400, Some(5120), 5120, "batch-job.exe", Some("C:\\Tools\\batch-job.exe"), me, 0.0, 220.0).state = ProcState::Suspended;
    b.add(SELF_PID, Some(5120), 5120, "ProcSight.exe", Some("C:\\Users\\user\\Desktop\\ProcSight.exe"), me, 1.1, 38.0).has_window = true;
    let testee = "C:\\work\\procsight\\target\\debug\\procsight-testee.exe";
    b.add(10_000, Some(5120), 5120, "procsight-testee.exe", Some(testee), me, 12.5, 4.0);
    let mut next = 10_100;
    for c in 0..3u32 {
        let child = 10_010 + c;
        b.add(child, Some(10_000), 10_000, "procsight-testee.exe", Some(testee), me, 0.0, 3.0);
        for _ in 0..3 {
            b.add(next, Some(child), child, "procsight-testee.exe", Some(testee), me, 0.0, 2.5);
            next += 1;
        }
    }
    b.add(6000, None, 5555, "orphan.exe", Some("C:\\Tools\\orphan.exe"), me, 0.0, 6.0);
    {
        let e = b.add(6100, Some(5120), 5120, "exiting.exe", Some("C:\\Tools\\exiting.exe"), me, 0.0, 1.0);
        e.state = ProcState::Exiting;
        e.threads = 0;
    }
    b.add(6200, Some(5120), 5120, "setup-helper.exe", Some("C:\\Tools\\setup-helper.exe"), me, 0.6, 12.0).is_new = 2;
    {
        let g = b.add(6300, Some(5120), 5120, "updater.exe", Some("C:\\Tools\\updater.exe"), me, 0.0, 9.0);
        g.is_gone = 1;
        g.cpu_pct = None;
        g.io_read_bps = None;
        g.io_write_bps = None;
    }
    // seq で少し動かす
    for r in b.rows.iter_mut() {
        if let Some(c) = r.cpu_pct.as_mut()
            && r.id.pid != 0
        {
            *c = (*c * (0.8 + noise(r.id.pid, seq as u32) * 0.4) * 10.0).round() / 10.0;
        }
    }
    finish(b.rows, seq)
}

fn wave(seq: u64, k: f32, base: f32, amp: f32) -> f32 {
    (base + amp * ((seq as f32 * 0.21 + k).sin() * 0.6 + (seq as f32 * 0.53 + k * 2.0).sin() * 0.4)).clamp(0.0, 100.0)
}

fn finish(rows: Vec<ProcessRow>, seq: u64) -> Snapshot {
    let totals = Totals {
        processes: rows.iter().filter(|r| r.is_gone == 0).count() as u32,
        threads: rows.iter().map(|r| r.threads).sum::<u32>() + 3600,
        handles: rows.iter().map(|r| r.handles as u64).sum::<u64>() + 120_000,
    };
    let touch_free = rows.iter().filter(|r| r.touch_free).map(|r| r.id).collect();
    let gb = 1024u64 * 1024 * 1024;
    let cpu = wave(seq, 0.0, 18.0, 10.0);
    let perf = PerfSample {
        cpu_total: Some(cpu),
        cpu_per_core: (0..8).map(|i| wave(seq, i as f32 * 0.9, cpu, 14.0)).collect(),
        mem: Some(MemSample {
            total: 32 * gb,
            available: 12 * gb + (wave(seq, 1.0, 50.0, 20.0) as u64) * gb / 100,
            commit_total: 21 * gb,
            commit_limit: 40 * gb,
        }),
        disks: Some(vec![
            DiskSample { name: "0 C:".into(), active_pct: wave(seq, 2.0, 6.0, 5.0), read_bps: wave(seq, 3.0, 20.0, 18.0) as f64 * 65_536.0, write_bps: wave(seq, 4.0, 10.0, 9.0) as f64 * 131_072.0 },
            DiskSample { name: "1 D:".into(), active_pct: wave(seq, 5.0, 1.0, 1.0), read_bps: 0.0, write_bps: 4096.0 },
        ]),
        nets: Some(vec![NetSample { name: "Wi-Fi".into(), send_bps: wave(seq, 6.0, 8.0, 7.0) as f64 * 4096.0, recv_bps: wave(seq, 7.0, 30.0, 25.0) as f64 * 40_960.0 }]),
        gpus: Some(vec![GpuSample {
            luid: "luid_0x00000000_0x0000D1A3".into(),
            name: "GPU 0".into(),
            util_pct: wave(seq, 8.0, 40.0, 25.0),
            dedicated_bytes: Some(5 * gb + gb / 4),
            shared_bytes: Some(gb / 3),
        }]),
        uptime_ms: (2 * 86_400 + 3 * 3600 + 14 * 60 + 55) * 1000 + seq * 1000,
    };
    Snapshot { seq, taken_at_ms: seq * 1000, logical_cpus: 8, rows, totals, perf, touch_free, rows_stale: false }
}

/// 行が `n` 個の Snapshot(NFR-4 の 1,000 行の確かめ)
pub fn big_snapshot(n: usize) -> Snapshot {
    let mut s = snapshot(1);
    let base = s.rows.clone();
    let mut pid = 20_000;
    while s.rows.len() < n {
        let mut r = base[s.rows.len() % base.len()].clone();
        r.id = ProcId { pid, create_time: BASE_FT + pid as u64 };
        r.parent = None;
        r.parent_pid = 0;
        r.touch_free = false;
        r.state = ProcState::Running;
        r.is_gone = 0;
        s.rows.push(r);
        pid += 4;
    }
    s
}

pub fn diagnostics() -> Diagnostics {
    Diagnostics {
        api: ApiAvailability { spi_times: true, spi_private_ws: true, spi_io: true, suspend: true, cmdline: true, handles: true, pdh_cpu: true, pdh_disk: true, pdh_gpu: true },
        collect_ms: (0..200).map(|i| 6.0 + noise(i, 1) * 8.0).collect(),
        late_in_row: 0,
        spi_fail_in_row: 0,
        open_counts: vec![(7000, 3), (5120, 1), (10_000, 2)],
        refused_touch_free: 4,
        timing: true,
    }
}

pub fn startup_items() -> Vec<StartupItem> {
    vec![
        StartupItem {
            name: "CloudSync".into(),
            location: StartupLocation::RegistryUser,
            command: "\"C:\\Users\\user\\AppData\\Local\\CloudSync\\cloudsync.exe\" /background".into(),
            exe_path: Some("C:\\Users\\user\\AppData\\Local\\CloudSync\\cloudsync.exe".into()),
            exe_exists: true,
            approved: Approved::Enabled,
            publisher: Some("Example Corporation".into()),
        },
        StartupItem {
            name: "Foo Helper".into(),
            location: StartupLocation::FolderUser,
            command: "C:\\Tools\\foo-helper.exe".into(),
            exe_path: Some("C:\\Tools\\foo-helper.exe".into()),
            exe_exists: false,
            approved: Approved::Disabled(BASE_FT - 36 * 3600 * 10_000_000),
            publisher: None,
        },
        StartupItem {
            name: "AudioTray".into(),
            location: StartupLocation::RegistryMachine,
            command: "\"C:\\Program Files\\Audio\\tray.exe\"".into(),
            exe_path: Some("C:\\Program Files\\Audio\\tray.exe".into()),
            exe_exists: true,
            approved: Approved::Unknown,
            publisher: Some("Sound Devices Ltd.".into()),
        },
    ]
}

pub fn services() -> Vec<ServiceRow> {
    let mk = |name: &str, dn: &str, state, st, pid, desc: &str, acc: &str, stop| ServiceRow {
        name: name.into(),
        display_name: dn.into(),
        state,
        start_type: Some(st),
        pid,
        description: desc.into(),
        account: acc.into(),
        accepts_stop: stop,
    };
    vec![
        mk("Spooler", "Print Spooler", ServiceState::Running, StartType::Auto, 2480, "印刷のジョブを待ち行列に入れます", "LocalSystem", true),
        mk("Audiosrv", "Windows Audio", ServiceState::Running, StartType::Auto, 1200, "音声を管理します", "NT AUTHORITY\\LocalService", true),
        mk("wuauserv", "Windows Update", ServiceState::Stopped, StartType::Manual, 0, "更新を探して入れます", "LocalSystem", false),
        mk("RpcSs", "Remote Procedure Call (RPC)", ServiceState::Running, StartType::Auto, 1040, "RPC の終点を扱います", "NT AUTHORITY\\NetworkService", false),
        mk("BITS", "Background Intelligent Transfer Service", ServiceState::StartPending, StartType::AutoDelayed, 0, "背景で転送します", "LocalSystem", true),
        mk("Fax", "Fax", ServiceState::Stopped, StartType::Disabled, 0, "ファクスを送受信します", "NT AUTHORITY\\NetworkService", true),
    ]
}

pub fn general() -> GeneralInfo {
    GeneralInfo {
        path: Ok(format!("{SYS32}\\notepad.exe")),
        command_line: Ok(format!("\"{SYS32}\\notepad.exe\" C:\\Users\\user\\Documents\\メモ.txt")),
        user: Ok("PC\\user".into()),
        elevated: Ok(false),
        integrity: Ok("中".into()),
        arch: Ok(Arch::X64),
        cpu_time: Ok(12_345 * 10_000_000),
        efficiency: Some(false),
    }
}

pub fn modules() -> Vec<ModuleRow> {
    let names = ["notepad.exe", "ntdll.dll", "kernel32.dll", "KERNELBASE.dll", "user32.dll", "gdi32.dll", "combase.dll", "uxtheme.dll"];
    names
        .iter()
        .enumerate()
        .map(|(i, n)| ModuleRow {
            name: n.to_string(),
            path: format!("{SYS32}\\{n}"),
            base: 0x7FF8_0000_0000 + i as u64 * 0x0020_0000,
            size: 0x4_0000 + i as u32 * 0x1_3000,
        })
        .collect()
}

pub fn handles() -> SharedHandleTable {
    let types = ["File", "Event", "Key", "Mutant", "Section", "Thread"];
    let rows: Vec<HandleRow> = (0..240u64)
        .map(|i| {
            let ty = types[(i % types.len() as u64) as usize];
            let name = match (ty, i % 7) {
                (_, 0) => HandleName::Pending,
                ("File", 3) => HandleName::TimedOut,
                ("File", _) => HandleName::Name(format!("C:\\Users\\user\\Documents\\file{i}.txt")),
                ("Event", _) => HandleName::Name(format!("\\Sessions\\1\\BaseNamedObjects\\ProcSightTest_{i}")),
                ("Key", _) => HandleName::Name("\\REGISTRY\\MACHINE\\SOFTWARE\\Example".into()),
                _ => HandleName::None,
            };
            HandleRow { value: 4 + i * 4, type_name: ty.into(), name, access: 0x001F_0003 }
        })
        .collect();
    let total = rows.len();
    Arc::new(Mutex::new(HandleTable { rows, names_done: total * 56 / 100, names_total: total, finished: false }))
}

pub fn threads() -> Vec<RawThread> {
    (0..9u32)
        .map(|i| RawThread {
            tid: 7004 + i * 4,
            start_address: 0x7FF8_0020_0000 + i as u64 * 0x1A0,
            priority: 8 + (i % 3) as i32,
            base_priority: 8,
            state: if i == 0 { 2 } else { 5 },
            wait_reason: if i == 1 { 5 } else { 6 },
            kernel_time: Some(i as u64 * 3_000_000),
            user_time: Some(i as u64 * 7_000_000),
            create_time: None,
        })
        .collect()
}

pub fn locking() -> Vec<LockingProcess> {
    vec![
        LockingProcess { pid: 7000, create_time: BASE_FT + 7000 * 10_000_000, app_name: "メモ帳".into(), kind: "アプリ".into(), service_short_name: None },
        LockingProcess { pid: 1200, create_time: 0, app_name: "Windows Audio".into(), kind: "サービス".into(), service_short_name: Some("Audiosrv".into()) },
    ]
}

/// 見本の画面の状態。CPU などのグラフは 60 回分たまった形
pub fn state() -> State {
    let mut cfg = Config::default();
    cfg.touch_free.names = vec!["acegame.exe".into()];
    cfg.columns.visible.push("threads".into());
    let mut st = State::new(env(), cfg);
    st.set_diagnostics(diagnostics());
    let notepad = snapshot(1).rows.iter().find(|r| r.name == "notepad.exe").unwrap().id;
    st.procs.select_only(notepad);
    for seq in 1..=60 {
        st.set_snapshot(Arc::new(snapshot(seq)));
    }
    st.procs.detail.general = Load::Ready(general());
    st.procs.detail.modules = Load::Ready(modules());
    st.procs.detail.handles = Load::Ready(handles());
    st.procs.detail.tab = DTab::General;
    st.threads.insert(notepad, threads());
    st.startup.items = Load::Ready(startup_items());
    st.startup.selected = Some(("CloudSync".into(), StartupLocation::RegistryUser));
    st.services.list = Load::Ready(services());
    st.services.selected = Some("Spooler".into());
    for p in [format!("{SYS32}\\notepad.exe"), format!("{SYS32}\\svchost.exe")] {
        st.sigs.insert(p, Some(Signature { status: SigStatus::Catalog, signer: Some("Microsoft Windows".into()), checked_at: BASE_FT }));
    }
    st.sigs.insert(
        "C:\\work\\procsight\\target\\debug\\procsight-testee.exe".into(),
        Some(Signature { status: SigStatus::Unsigned, signer: None, checked_at: BASE_FT }),
    );
    st.replace = Some(ReplaceInfo {
        debugger: None,
        debugger_wow64: None,
        self_final_path: st.env.self_final_path.clone(),
        block: Some(procsight_core::taskmgr::ReplaceBlock::NotElevated),
    });
    st.actions.clear();
    st.config_dirty = false;
    st.suspended_dirty = false;
    st
}
