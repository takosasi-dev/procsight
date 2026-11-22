//! 公開されていない NT の API と構造体(仕様書 §4 事実19〜事実24)。1 つにつき 1 つのラッパーを置き、
//! 取れない・確かめられなかった物は機能を隠して動き続ける(D-6)。windows crate に無い関数は ntdll から探す。

pub mod cmdline;
pub mod handles;
pub mod spi;
pub mod suspend;

use crate::util::{pcwstr, wide};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::PCSTR;

/// ntdll.dll の関数の位置を探す。`name` は 0 で終わる ASCII(例: b"NtSuspendProcess\0")
pub fn ntdll_proc(name: &'static [u8]) -> Option<unsafe extern "system" fn() -> isize> {
    debug_assert_eq!(name.last(), Some(&0));
    let module = wide("ntdll.dll");
    // SAFETY: ntdll.dll はどのプロセスにも読み込まれていて、解放されない。名前は 0 で終わる
    unsafe {
        let h = GetModuleHandleW(pcwstr(&module)).ok()?;
        GetProcAddress(h, PCSTR(name.as_ptr()))
    }
}

/// NTSTATUS の STATUS_INFO_LENGTH_MISMATCH
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;
