//! クリップボードの文字の読み書き(FR-36)。eframe の clipboard の機能は arboard(BSL-1.0)を引き込み INV-11 の外なので、
//! Win32 で自前に行う。画面のスレッドから呼んでよい(INV-9 の例外。すぐ返る呼び出しだけ)。

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};

const CF_UNICODETEXT: u32 = 13;

struct Opened;
impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: OpenClipboard が成功した後にだけ作り、1 回だけ閉じる
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn open() -> Option<Opened> {
    for _ in 0..5 {
        // SAFETY: 窓を指定せずに開く。成功したら Opened の Drop で閉じる
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Some(Opened);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let Some(_g) = open() else { return false };
    // SAFETY: 確保した大きさの中にだけ書く。SetClipboardData が成功したらメモリの持ち主はクリップボードになり、
    // 失敗したときだけ自分で解放する
    unsafe {
        if EmptyClipboard().is_err() {
            return false;
        }
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else { return false };
        let p = GlobalLock(mem) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(Some(mem));
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
            return false;
        }
    }
    true
}

pub fn get_text() -> Option<String> {
    // SAFETY: 形式があるかを聞くだけ
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_err() {
        return None;
    }
    let _g = open()?;
    // SAFETY: GetClipboardData のメモリはクリップボードの物で、閉じるまで有効。GlobalSize の長さの中だけを読む
    unsafe {
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let mem = HGLOBAL(h.0);
        let p = GlobalLock(mem) as *const u16;
        if p.is_null() {
            return None;
        }
        let len = GlobalSize(mem) / 2;
        let s = std::slice::from_raw_parts(p, len);
        let text = crate::util::from_wide(s);
        let _ = GlobalUnlock(mem);
        Some(text)
    }
}
