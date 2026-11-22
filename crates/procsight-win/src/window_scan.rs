//! 窓の一覧(担当 A。D-32)。使うのは EnumWindows・GetWindowThreadProcessId・IsWindowVisible・IsHungAppWindow だけ(D-7)。
//! 窓へメッセージを送らない(INV-1)。

use procsight_core::WindowInfo;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsHungAppWindow, IsWindowVisible};
use windows::core::BOOL;

extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: lp は visible_windows が渡した Vec<(HWND, u32)> を指し、EnumWindows が返るまで生きている
    let out = unsafe { &mut *(lp.0 as *mut Vec<(HWND, u32)>) };
    // SAFETY: hwnd は EnumWindows が渡したトップレベルの窓。読むだけの関数
    unsafe {
        if IsWindowVisible(hwnd).as_bool() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            out.push((hwnd, pid));
        }
    }
    BOOL(1)
}

/// 見えているトップレベルの窓と、その PID(EnumWindows の順)
fn visible_windows() -> Vec<(HWND, u32)> {
    let mut out: Vec<(HWND, u32)> = Vec::with_capacity(256);
    // SAFETY: collect は out を指す LPARAM だけを使い、EnumWindows はこの呼び出しの中でだけ collect を呼ぶ
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut out as *mut _ as isize)) };
    out
}

/// 見えているトップレベルの窓を持つ PID と、応答なしの PID
pub fn scan() -> WindowInfo {
    let mut info = WindowInfo::default();
    for (hwnd, pid) in visible_windows() {
        info.visible.insert(pid);
        // SAFETY: OS が持っている状態を読むだけ(事実13)
        if unsafe { IsHungAppWindow(hwnd) }.as_bool() {
            info.hung.insert(pid);
        }
    }
    info
}

/// その PID の見えているトップレベルの窓のうち最初の物(FR-17)
pub fn first_visible_window(pid: u32) -> Option<HWND> {
    visible_windows().into_iter().find(|(_, p)| *p == pid).map(|(h, _)| h)
}

#[cfg(test)]
mod tests {
    #[test]
    fn scan_does_not_panic() {
        let w = super::scan();
        assert!(w.hung.is_subset(&w.visible));
    }
}
