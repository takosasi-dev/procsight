//! NtSuspendProcess / NtResumeProcess(担当 A。D-14・事実21)。ntdll から探し、無ければ一時停止を画面から隠す。
//! ハンドルは呼ぶ側が関門で PROCESS_SUSPEND_RESUME を付けて開いた物を渡す。

use crate::nt::ntdll_proc;
use std::sync::OnceLock;
use windows::Win32::Foundation::HANDLE;

type NtProcessFn = unsafe extern "system" fn(HANDLE) -> i32;

fn lookup(name: &'static [u8]) -> Option<NtProcessFn> {
    let f = ntdll_proc(name)?;
    // SAFETY: NtSuspendProcess / NtResumeProcess は (HANDLE) -> NTSTATUS の形(事実21。probe で確かめる)。
    // 関数のポインタの型を付け直すだけで、大きさは同じ
    Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, NtProcessFn>(f) })
}

fn suspend_fn() -> Option<NtProcessFn> {
    static F: OnceLock<Option<NtProcessFn>> = OnceLock::new();
    *F.get_or_init(|| lookup(b"NtSuspendProcess\0"))
}

fn resume_fn() -> Option<NtProcessFn> {
    static F: OnceLock<Option<NtProcessFn>> = OnceLock::new();
    *F.get_or_init(|| lookup(b"NtResumeProcess\0"))
}

/// 2 つとも取れたときだけ一時停止を出す
pub fn available() -> bool {
    suspend_fn().is_some() && resume_fn().is_some()
}

fn call(f: Option<NtProcessFn>, h: HANDLE) -> Result<(), i32> {
    let f = f.ok_or(0xC000_0002u32 as i32)?; // STATUS_NOT_IMPLEMENTED
    // SAFETY: f は ntdll の関数で、h は呼ぶ側が PROCESS_SUSPEND_RESUME で開いた有効なハンドル
    let status = unsafe { f(h) };
    if status >= 0 { Ok(()) } else { Err(status) }
}

/// 失敗したら NTSTATUS
pub fn suspend(h: HANDLE) -> Result<(), i32> {
    call(suspend_fn(), h)
}

pub fn resume(h: HANDLE) -> Result<(), i32> {
    call(resume_fn(), h)
}
