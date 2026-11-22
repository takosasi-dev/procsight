//! この crate の中で共有する小道具: Win32 のエラー番号の取り出し、UTF-16 との変換、FILETIME、
//! FormatMessageW の文、自分の PID・System のフォルダ・Program Files・論理プロセッサ数などの環境の値。

use windows::Win32::Foundation::FILETIME;
use windows::core::{Error, PCWSTR, PWSTR};

/// windows::core::Error から Win32 のエラー番号を取り出す(HRESULT_FROM_WIN32 の形なら下 16 ビット)
pub fn win32_code(e: &Error) -> u32 {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 { hr & 0xFFFF } else { hr }
}

/// 末尾に 0 を付けた UTF-16
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `wide` の結果を PCWSTR として渡す。v は呼び出しの間生きていること
pub fn pcwstr(v: &[u16]) -> PCWSTR {
    PCWSTR(v.as_ptr())
}

/// 0 で終わる UTF-16 のバッファから String に(0 が無ければ全部)
pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// PWSTR(0 で終わる)を String に。null なら空
pub fn pwstr_to_string(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: 呼ぶ側が、p が 0 で終わる有効な UTF-16 の文字列を指すことを保証する(Win32 が返した物だけを渡す)
    unsafe { p.to_string().unwrap_or_default() }
}

pub fn filetime_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 今の時刻の FILETIME
pub fn now_filetime() -> u64 {
    // SAFETY: 引数なし。戻り値を受けるだけ
    filetime_u64(unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() })
}

/// FormatMessageW の文(§9.12 の「その他」)
pub fn error_text(code: u32) -> String {
    use windows::Win32::System::Diagnostics::Debug::{
        FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
    };
    let mut buf = [0u16; 512];
    // SAFETY: buf はこの関数の中の配列で、長さを渡している。挿入の引数は使わない(IGNORE_INSERTS)
    let n = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            code,
            0,
            PWSTR(buf.as_mut_ptr()),
            buf.len() as u32,
            None,
        )
    };
    String::from_utf16_lossy(&buf[..n as usize]).trim().to_string()
}

pub fn self_pid() -> u32 {
    std::process::id()
}

/// Windows のフォルダ(例: C:\Windows)。環境変数はユーザーが書き換えられる(HKCU\Environment)ので OS から聞く
pub fn system_root() -> String {
    use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    let mut buf = [0u16; 260];
    // SAFETY: buf はこの関数の中の配列で、長さを渡している
    let n = unsafe { GetSystemWindowsDirectoryW(Some(&mut buf)) } as usize;
    if n == 0 || n >= buf.len() { "C:\\Windows".into() } else { String::from_utf16_lossy(&buf[..n]) }
}

/// Program Files(64 ビットの方)。D-39 の判定に使うので、環境変数ではなく既知のフォルダの API で聞く(空なら判定で断る)
pub fn program_files() -> String {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: 返った文字列は CoTaskMemFree で 1 回だけ解放する
    unsafe {
        match SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None) {
            Ok(p) => {
                let s = pwstr_to_string(p);
                CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
                s
            }
            Err(_) => String::new(),
        }
    }
}

/// 環境変数を展開する(ExpandEnvironmentStringsW)
pub fn expand_env(s: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    let src = wide(s);
    let mut buf = vec![0u16; 1024];
    // SAFETY: src は 0 で終わる。buf の長さを渡し、必要な長さが返ったら取り直す
    let n = unsafe { ExpandEnvironmentStringsW(pcwstr(&src), Some(&mut buf)) } as usize;
    if n > buf.len() {
        buf = vec![0u16; n];
        // SAFETY: 同上
        unsafe { ExpandEnvironmentStringsW(pcwstr(&src), Some(&mut buf)) };
    }
    if n == 0 { s.to_string() } else { from_wide(&buf) }
}

/// UTC からのずれ(分)。日付の表示に使う
pub fn local_offset_minutes() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    // SAFETY: tz はこの関数の中の構造体
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = match r {
        2 => tz.Bias + tz.DaylightBias, // TIME_ZONE_ID_DAYLIGHT
        _ => tz.Bias + tz.StandardBias,
    };
    -bias
}

/// 単調な時計(ミリ秒)
pub fn tick_ms() -> u64 {
    // SAFETY: 引数なし
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// 論理プロセッサの数(全グループ)
pub fn logical_cpus() -> u32 {
    use windows::Win32::System::Threading::{ALL_PROCESSOR_GROUPS, GetActiveProcessorCount};
    // SAFETY: 引数の値だけを読む
    unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1)
}

/// プロセッサのグループの数(D-16)
pub fn processor_groups() -> u16 {
    // SAFETY: 引数なし
    unsafe { windows::Win32::System::Threading::GetActiveProcessorGroupCount() }
}

/// CPU の名前(HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0 の ProcessorNameString。読むだけ)
pub fn cpu_name() -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let key = wide("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0");
    let name = wide("ProcessorNameString");
    let mut buf = vec![0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: key と name は 0 で終わる。buf の大きさ(バイト)を len で渡す
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            pcwstr(&key),
            pcwstr(&name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    (r.0 == 0).then(|| from_wide(&buf).trim().to_string()).filter(|s| !s.is_empty())
}

/// 窓を作る前・作れなかったときの知らせ(E-34・終了コード 3)。画面のスレッドから呼んでよい(INV-9 の例外: 起動と終了の時だけ)
pub fn message_box(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    let t = wide(text);
    let c = wide("ProcSight");
    // SAFETY: t と c は 0 で終わり、呼び出しの間生きている。親の窓は無し
    unsafe {
        MessageBoxW(None, pcwstr(&t), pcwstr(&c), MB_OK | MB_ICONWARNING);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_round_trip() {
        assert_eq!(from_wide(&wide("プロセス")), "プロセス");
        assert_eq!(win32_code(&Error::from_hresult(windows::core::HRESULT(0x8007_0005u32 as i32))), 5);
        assert!(logical_cpus() >= 1);
        assert!(!system_root().is_empty());
        assert!(error_text(5).len() > 1);
        assert!(cpu_name().is_some());
        assert!(program_files().to_lowercase().ends_with("program files"), "{}", program_files());
        assert!(system_root().to_lowercase().ends_with("windows"));
    }
}
