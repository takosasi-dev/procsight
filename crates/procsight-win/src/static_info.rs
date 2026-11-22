//! 新しいプロセスごとに 1 回取る値(担当 A。D-19)。パス・ユーザー・昇格・重要プロセスの印・アーキテクチャ。
//! 関門で PROCESS_QUERY_LIMITED_INFORMATION を開いて取り、すぐ閉じる。
//! 自分の昇格(擬似ハンドル)と Windows の版(RtlGetVersion)もここで読む。

use crate::gate::{GateError, ProcessGate};
use crate::util::from_wide;
use procsight_core::{Arch, InfoError, ProcId, StaticInfo};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
    TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
    IMAGE_FILE_MACHINE_UNKNOWN, OSVERSIONINFOW,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, IsProcessCritical, IsWow64Process2, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

pub fn info_error(e: GateError) -> InfoError {
    match e {
        GateError::TouchFree => InfoError::TouchFree,
        GateError::Replaced => InfoError::Replaced,
        GateError::Gone => InfoError::Gone,
        GateError::AccessDenied => InfoError::AccessDenied,
        GateError::Protected => InfoError::Protected,
        GateError::Other(c) => InfoError::Other(c),
    }
}

pub fn fetch(gate: &ProcessGate, id: ProcId) -> StaticInfo {
    let p = match gate.open(id, PROCESS_QUERY_LIMITED_INFORMATION) {
        Ok(p) => p,
        Err(e) => return StaticInfo { error: Some(info_error(e)), ..Default::default() },
    };
    let h = p.raw();
    let (user, elevated) = with_token(h, |t| (token_user(t), token_elevated(t))).unwrap_or((None, None));
    StaticInfo { path: image_path(h), user, elevated, critical: is_critical(h), arch: arch(h), error: None }
}

/// QueryFullProcessImageNameW(Win32 の形)
pub fn image_path(h: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32 * 1024];
    let mut len = buf.len() as u32;
    // SAFETY: h は有効なハンドル。buf は len 文字の書ける領域
    unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// トークンを開いて f を呼び、閉じる
fn with_token<T>(h: HANDLE, f: impl FnOnce(HANDLE) -> T) -> Option<T> {
    let mut tok = HANDLE::default();
    // SAFETY: h は有効なハンドル。tok はこの関数の変数で、下で 1 回だけ閉じる
    unsafe { OpenProcessToken(h, TOKEN_QUERY, &mut tok) }.ok()?;
    let r = f(tok);
    // SAFETY: tok は今開いたトークン
    unsafe {
        let _ = CloseHandle(tok);
    }
    Some(r)
}

/// GetTokenInformation の結果を 8 バイトに揃ったバッファで返す
fn token_info(tok: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    let mut need = 0u32;
    // SAFETY: 大きさだけを聞く(バッファなし)。失敗(ERROR_INSUFFICIENT_BUFFER)は想定どおり
    let _ = unsafe { GetTokenInformation(tok, class, None, 0, &mut need) };
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u64; (need as usize).div_ceil(8)];
    // SAFETY: buf は need バイト以上の書ける領域
    unsafe { GetTokenInformation(tok, class, Some(buf.as_mut_ptr().cast()), need, &mut need) }.ok()?;
    Some(buf)
}

/// 「ドメイン\名前」。ログと設定には書かない(INV-7)
fn token_user(tok: HANDLE) -> Option<String> {
    let buf = token_info(tok, TokenUser)?;
    // SAFETY: buf は GetTokenInformation(TokenUser) が埋めた TOKEN_USER で、8 バイトに揃っている。Sid は buf の中を指す
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let (mut name, mut domain) = ([0u16; 256], [0u16; 256]);
    let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
    let mut use_ = SID_NAME_USE::default();
    // SAFETY: sid は buf が生きている間有効。name・domain は長さを渡した配列
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut nl,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut dl,
            &mut use_,
        )
    }
    .ok()?;
    let (n, d) = (from_wide(&name), from_wide(&domain));
    Some(if d.is_empty() { n } else { format!("{d}\\{n}") })
}

fn token_elevated(tok: HANDLE) -> Option<bool> {
    let buf = token_info(tok, TokenElevation)?;
    // SAFETY: buf は GetTokenInformation(TokenElevation) が埋めた TOKEN_ELEVATION
    Some(unsafe { (*(buf.as_ptr() as *const TOKEN_ELEVATION)).TokenIsElevated } != 0)
}

pub(crate) fn is_critical(h: HANDLE) -> Option<bool> {
    let mut b = BOOL::default();
    // SAFETY: h は有効なハンドル。b はこの関数の変数
    unsafe { IsProcessCritical(h, &mut b) }.ok()?;
    Some(b.as_bool())
}

fn arch(h: HANDLE) -> Option<Arch> {
    let (mut pm, mut nm) = (IMAGE_FILE_MACHINE::default(), IMAGE_FILE_MACHINE::default());
    // SAFETY: h は有効なハンドル。pm・nm はこの関数の変数
    unsafe { IsWow64Process2(h, &mut pm, Some(&mut nm)) }.ok()?;
    // WOW64 でなければ process の側は UNKNOWN で、OS と同じ
    let m = if pm == IMAGE_FILE_MACHINE_UNKNOWN { nm } else { pm };
    Some(match m {
        IMAGE_FILE_MACHINE_AMD64 => Arch::X64,
        IMAGE_FILE_MACHINE_I386 => Arch::X86,
        IMAGE_FILE_MACHINE_ARM64 => Arch::Arm64,
        other => Arch::Other(other.0),
    })
}

/// 自分が昇格しているか(GetCurrentProcess の擬似ハンドル。関門を通さない)。
/// elevation::is_elevated(担当 B)と同じ問いだが、Backend の起動を B の実装に依らせないためにここで読む
pub fn self_elevated() -> bool {
    // SAFETY: GetCurrentProcess は閉じる必要のない擬似ハンドル
    let me = unsafe { GetCurrentProcess() };
    with_token(me, token_elevated).flatten().unwrap_or(false)
}

/// (メジャー, マイナー, ビルド)。RtlGetVersion(GetVersionEx と違い、マニフェストで値が変わらない)
pub fn windows_version() -> Option<(u32, u32, u32)> {
    let f = crate::nt::ntdll_proc(b"RtlGetVersion\0")?;
    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
    // SAFETY: RtlGetVersion は (PRTL_OSVERSIONINFOW) -> NTSTATUS の形(文書あり)。型を付け直すだけ
    let f = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, RtlGetVersion>(f) };
    let mut v = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    // SAFETY: v はこの関数の変数で、大きさを入れてある
    (unsafe { f(&mut v) } >= 0).then_some((v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_self() {
        let me = std::process::id();
        let ct = crate::nt::spi::query_processes().unwrap().into_iter().find(|p| p.id.pid == me).unwrap().id.create_time;
        let g = ProcessGate::new(me, self_elevated());
        let s = fetch(&g, ProcId { pid: me, create_time: ct });
        assert_eq!(s.error, None);
        assert!(s.path.as_deref().unwrap().to_lowercase().ends_with(".exe"));
        assert!(s.user.is_some());
        assert_eq!(s.critical, Some(false));
        assert_eq!(s.arch, Some(Arch::X64));
        assert_eq!(s.elevated, Some(self_elevated()));
        assert!(windows_version().unwrap().2 >= 22000);
    }
}
