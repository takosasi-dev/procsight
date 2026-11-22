//! プロセスへの操作(担当 A。FR-10〜FR-17)。終了・優先度・割り当て・効率モード・一時停止・再開・前面に出す。
//! どれも関門で開いて開始時刻を確かめてから行う(INV-6)。終了・一時停止を断る判定(rules)は、
//! Worker が直前の Snapshot の行で `check_row` を通してから呼ぶ。

use crate::gate::{GateError, ProcessGate};
use crate::jobs::{OpError, Target};
use crate::nt::suspend;
use crate::util::win32_code;
use procsight_core::rules::{self, Refusal};
use procsight_core::{Priority, ProcId, Snapshot};
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetActiveProcessorGroupCount, GetProcessAffinityMask,
    GetProcessInformation, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_CREATION_FLAGS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
    ProcessPowerThrottling, SetPriorityClass, SetProcessAffinityMask, SetProcessInformation, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};

pub const MULTI_GROUP: &str = "この PC では使えません(プロセッサのグループが複数)";

fn win(e: windows::core::Error) -> OpError {
    OpError::from_code(win32_code(&e))
}

/// 終了・一時停止の直前の判定(INV-4)。今の Snapshot に行が無ければ「すでに終了しています」
pub fn check_row(snap: &Snapshot, id: ProcId, suspend: bool, self_pid: u32, system_root: &str) -> Result<(), OpError> {
    let row = snap.find(id).ok_or(OpError::Refused(Refusal::Gone))?;
    let r = if suspend {
        rules::check_suspend(row, self_pid, system_root)
    } else {
        rules::check_terminate(row, self_pid, system_root)
    };
    r.map_err(OpError::Refused)
}

/// INV-4 の念押し。StaticInfo をまだ取っていない行(起動直後・最小化から戻った直後)は rules が重要プロセスを見分けられないので、
/// 開いた同じハンドル(関門が QUERY_LIMITED を足している)で IsProcessCritical を読んで断る
fn refuse_critical(p: &crate::gate::OwnedProcess) -> Result<(), OpError> {
    if crate::static_info::is_critical(p.raw()) == Some(true) {
        return Err(OpError::Refused(Refusal::Critical));
    }
    Ok(())
}

/// FR-10。TerminateProcess(h, 1)
pub fn terminate(gate: &ProcessGate, id: ProcId) -> Result<(), OpError> {
    let p = gate.open(id, PROCESS_TERMINATE)?;
    refuse_critical(&p)?;
    // SAFETY: p は関門で PROCESS_TERMINATE を付けて開き、開始時刻を確かめたハンドル
    unsafe { TerminateProcess(p.raw(), 1) }.map_err(win)
}

fn class(p: Priority) -> PROCESS_CREATION_FLAGS {
    match p {
        Priority::Idle => IDLE_PRIORITY_CLASS,
        Priority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        Priority::Normal => NORMAL_PRIORITY_CLASS,
        Priority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        Priority::High => HIGH_PRIORITY_CLASS,
    }
}

/// FR-12。5 つの値だけ(リアルタイムは型に無い。INV-14)
pub fn set_priority(gate: &ProcessGate, id: ProcId, p: Priority) -> Result<(), OpError> {
    let h = gate.open(id, PROCESS_SET_INFORMATION)?;
    // SAFETY: h は関門で開いたハンドル
    unsafe { SetPriorityClass(h.raw(), class(p)) }.map_err(win)
}

fn single_group() -> Result<(), OpError> {
    // SAFETY: 引数なし
    if unsafe { GetActiveProcessorGroupCount() } == 1 { Ok(()) } else { Err(OpError::Blocked(MULTI_GROUP)) }
}

/// FR-13。(プロセスのマスク, システムのマスク)。プロセッサのグループが 1 つのときだけ(D-16)
pub fn get_affinity(gate: &ProcessGate, id: ProcId) -> Result<(u64, u64), OpError> {
    single_group()?;
    let h = gate.open(id, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let (mut pm, mut sm) = (0usize, 0usize);
    // SAFETY: h は関門で開いたハンドル。pm・sm はこの関数の変数
    unsafe { GetProcessAffinityMask(h.raw(), &mut pm, &mut sm) }.map_err(win)?;
    Ok((pm as u64, sm as u64))
}

pub fn set_affinity(gate: &ProcessGate, id: ProcId, mask: u64) -> Result<(), OpError> {
    single_group()?;
    if mask == 0 {
        return Err(OpError::Blocked("論理プロセッサを 1 つ以上選んでください"));
    }
    let h = gate.open(id, PROCESS_SET_INFORMATION)?;
    // SAFETY: h は関門で開いたハンドル
    unsafe { SetProcessAffinityMask(h.raw(), mask as usize) }.map_err(win)
}

/// D-15。入れる = 「低」+ EcoQoS。切る = 「通常」+ OS の管理に戻す(ControlMask = 0)
pub fn set_efficiency(gate: &ProcessGate, id: ProcId, on: bool) -> Result<(), OpError> {
    let h = gate.open(id, PROCESS_SET_INFORMATION)?;
    let speed = if on { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 };
    let state = PROCESS_POWER_THROTTLING_STATE { Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION, ControlMask: speed, StateMask: speed };
    // SAFETY: h は関門で開いたハンドル。state はこの関数の変数で、大きさを渡す
    unsafe {
        SetPriorityClass(h.raw(), if on { IDLE_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS }).map_err(win)?;
        SetProcessInformation(
            h.raw(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .map_err(win)
    }
}

/// FR-14。EcoQoS が入っているか。読めなければ None(E-19)
pub fn efficiency(gate: &ProcessGate, id: ProcId) -> Option<bool> {
    let h = gate.open(id, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    let mut state = PROCESS_POWER_THROTTLING_STATE { Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION, ..Default::default() };
    // SAFETY: h は関門で開いたハンドル。state はこの関数の変数で、大きさを渡す
    unsafe {
        GetProcessInformation(
            h.raw(),
            ProcessPowerThrottling,
            (&mut state as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .ok()?;
    Some(state.ControlMask & state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0)
}

/// NTSTATUS を OpError に。よく出る物だけを直し、他は RtlNtStatusToDosError の番号
fn nt_error(status: i32) -> OpError {
    match status as u32 {
        0xC000_0022 => OpError::Win32(5),                // STATUS_ACCESS_DENIED
        0xC000_010A => OpError::Gate(GateError::Gone),   // STATUS_PROCESS_IS_TERMINATING
        0xC000_0002 => OpError::Unsupported,             // STATUS_NOT_IMPLEMENTED(関数が無い)
        _ => {
            type ToDos = unsafe extern "system" fn(i32) -> u32;
            match crate::nt::ntdll_proc(b"RtlNtStatusToDosError\0") {
                Some(f) => {
                    // SAFETY: RtlNtStatusToDosError は (NTSTATUS) -> ULONG の形(winternl.h に文書あり)。型を付け直すだけ
                    let f = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, ToDos>(f) };
                    // SAFETY: 値を渡すだけ
                    OpError::Win32(unsafe { f(status) })
                }
                None => OpError::Win32(status as u32),
            }
        }
    }
}

/// FR-15・D-14。NtSuspendProcess
pub fn suspend(gate: &ProcessGate, id: ProcId) -> Result<(), OpError> {
    if !suspend::available() {
        return Err(OpError::Unsupported);
    }
    let h = gate.open(id, PROCESS_SUSPEND_RESUME)?;
    refuse_critical(&h)?;
    suspend::suspend(h.raw()).map_err(nt_error)
}

pub fn resume(gate: &ProcessGate, id: ProcId) -> Result<(), OpError> {
    if !suspend::available() {
        return Err(OpError::Unsupported);
    }
    let h = gate.open(id, PROCESS_SUSPEND_RESUME)?;
    suspend::resume(h.raw()).map_err(nt_error)
}

/// FR-17。触れないプロセスでないことを関門の permit で確かめてから SetForegroundWindow を呼ぶ
#[allow(clippy::disallowed_methods)] // FR-17 の「前面に出す」はこの関数だけ。permit で触れないプロセスを断った後に呼ぶ(INV-1)
pub fn bring_to_front(gate: &ProcessGate, target: &Target) -> Result<(), OpError> {
    // 開いて開始時刻を確かめ、持ったまま窓を探す(持っている間は PID が使い回されない。INV-6)
    let _hold = gate.open(target.id, windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION)?;
    gate.permit(target.id, "bring_to_front")?;
    let hwnd = crate::window_scan::first_visible_window(target.id.pid)
        .ok_or(OpError::Blocked("このプロセスには見えている窓がありません"))?;
    // SAFETY: hwnd は今 EnumWindows で見つけた窓。窓がもう無ければ関数は失敗を返すだけ
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            Ok(())
        } else {
            Err(OpError::Blocked("Windows が前面への切り替えを許しませんでした"))
        }
    }
}
