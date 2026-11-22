//! 昇格の確かめと、管理者で再起動(担当 B。FR-45・D-18)。トークンの読み取り(ユーザー・昇格・整合性レベル)の
//! 補助もここに置き、details::general からも使う。

use crate::jobs::OpError;
use crate::util::{pcwstr, wide, win32_code};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, LookupAccountSidW, SID_NAME_USE,
    TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenElevation,
    TokenIntegrityLevel, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::core::{PCWSTR, PWSTR};

/// プロセスのトークン。Drop で閉じる
pub(crate) struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: OpenProcessToken が返した有効なハンドルで、この型だけが持ち、1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `process` は自分の擬似ハンドルか、関門で開いたハンドル
pub(crate) fn open_token(process: HANDLE) -> Result<Token, u32> {
    let mut h = HANDLE::default();
    // SAFETY: process は呼ぶ側が有効と保証する。h はこの関数の中の変数
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut h) }.map_err(|e| win32_code(&e))?;
    Ok(Token(h))
}

/// GetTokenInformation の結果を 8 バイトに揃えたバッファで返す
fn token_info(t: &Token, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<u64>, u32> {
    let mut need = 0u32;
    // SAFETY: 大きさを聞くだけ(バッファなし)。失敗は想定どおり(ERROR_INSUFFICIENT_BUFFER)
    let _ = unsafe { GetTokenInformation(t.0, class, None, 0, &mut need) };
    if need == 0 {
        return Err(87);
    }
    let mut buf = vec![0u64; (need as usize).div_ceil(8)];
    // SAFETY: buf は need バイト以上あり、長さを渡している
    unsafe { GetTokenInformation(t.0, class, Some(buf.as_mut_ptr().cast()), need, &mut need) }
        .map_err(|e| win32_code(&e))?;
    Ok(buf)
}

/// 「ドメイン\名前」
pub(crate) fn token_user(t: &Token) -> Result<String, u32> {
    let buf = token_info(t, TokenUser)?;
    // SAFETY: TokenUser の結果は先頭が TOKEN_USER で、Sid は同じバッファの中を指す。buf はこの後も生きている
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let (mut name, mut domain) = (vec![0u16; 256], vec![0u16; 256]);
    let (mut n, mut d) = (name.len() as u32, domain.len() as u32);
    let mut use_ = SID_NAME_USE::default();
    // SAFETY: sid は上のバッファの中。name・domain は長さを渡した配列
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut n,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut d,
            &mut use_,
        )
    }
    .map_err(|e| win32_code(&e))?;
    let (name, domain) = (crate::util::from_wide(&name), crate::util::from_wide(&domain));
    Ok(if domain.is_empty() { name } else { format!("{domain}\\{name}") })
}

pub(crate) fn token_elevated(t: &Token) -> Result<bool, u32> {
    let buf = token_info(t, TokenElevation)?;
    // SAFETY: TokenElevation の結果は TOKEN_ELEVATION(4 バイト)。buf は 8 バイト以上
    Ok(unsafe { (*(buf.as_ptr() as *const TOKEN_ELEVATION)).TokenIsElevated } != 0)
}

/// 整合性レベルの RID(SECURITY_MANDATORY_*_RID)
pub(crate) fn token_integrity_rid(t: &Token) -> Result<u32, u32> {
    let buf = token_info(t, TokenIntegrityLevel)?;
    // SAFETY: 結果は TOKEN_MANDATORY_LABEL で、Sid は同じバッファの中。最後の下位権限が RID
    unsafe {
        let sid = (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(87);
        }
        Ok(*GetSidSubAuthority(sid, count as u32 - 1))
    }
}

/// RID を画面の言葉に(SECURITY_MANDATORY_LOW_RID 0x1000、MEDIUM 0x2000、HIGH 0x3000、SYSTEM 0x4000、PROTECTED_PROCESS 0x5000)
pub fn integrity_label(rid: u32) -> &'static str {
    match rid {
        0..0x1000 => "信頼されていない",
        0x1000..0x2000 => "低",
        0x2000..0x3000 => "中",
        0x3000..0x4000 => "高",
        0x4000..0x5000 => "システム",
        _ => "保護されたプロセス",
    }
}

fn self_token() -> Option<Token> {
    // SAFETY: GetCurrentProcess は自分の擬似ハンドルを返すだけ(閉じなくてよい)
    open_token(unsafe { GetCurrentProcess() }).ok()
}

pub fn is_elevated() -> bool {
    self_token().and_then(|t| token_elevated(&t).ok()).unwrap_or(false)
}

/// ProcSight を動かしているユーザー(ドメイン\名前)。ログと設定には書かない(INV-7)
pub fn current_user() -> Option<String> {
    self_token().and_then(|t| token_user(&t).ok())
}

/// `\\?\` と `\\?\UNC\` を外す
pub(crate) fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else {
        p.strip_prefix("\\\\?\\").unwrap_or(p).to_string()
    }
}

/// 今の exe の実体のパス(GetFinalPathNameByHandleW。\\?\ を外す)。std の canonicalize が中で
/// GetFinalPathNameByHandleW を呼ぶので、それを使う
pub fn self_final_path() -> String {
    let Ok(exe) = std::env::current_exe() else { return String::new() };
    let p = std::fs::canonicalize(&exe).unwrap_or(exe);
    strip_verbatim(&p.to_string_lossy())
}

/// ShellExecuteExW に渡す、昇格した自分の引数(D-18)
fn restart_args(pid: u32) -> String {
    format!("--after-restart {pid}")
}

/// ShellExecuteExW の runas で自分を `--after-restart <自分の PID>` 付きで起動する。成功したら呼ぶ側が終わる
pub fn restart_elevated() -> Result<(), OpError> {
    let exe = std::env::current_exe().map_err(|e| OpError::from_code(e.raw_os_error().unwrap_or(2) as u32))?;
    let file = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params = wide(&restart_args(crate::util::self_pid()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: pcwstr(&verb),
        lpFile: pcwstr(&file),
        lpParameters: pcwstr(&params),
        nShow: 1, // SW_SHOWNORMAL
        ..Default::default()
    };
    // SAFETY: info の文字列はこの関数の中の配列を指し、呼び出しの間生きている
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| OpError::from_code(win32_code(&e)))
}

/// 動いているプロセスの PID の一覧(EnumProcesses。どのプロセスも開かない)
fn running_pids() -> Vec<u32> {
    use windows::Win32::System::ProcessStatus::EnumProcesses;
    let mut buf = vec![0u32; 1024];
    loop {
        let mut used = 0u32;
        // SAFETY: buf の大きさ(バイト)を渡し、書かれた分だけを読む
        if unsafe { EnumProcesses(buf.as_mut_ptr(), (buf.len() * 4) as u32, &mut used) }.is_err() {
            return Vec::new();
        }
        let n = used as usize / 4;
        if n < buf.len() {
            buf.truncate(n);
            return buf;
        }
        buf = vec![0u32; buf.len() * 2];
    }
}

/// その PID のプロセスが終わるのを最大 timeout_ms 待つ(--after-restart)。ハンドルを開かずに EnumProcesses を
/// 100 ミリ秒ごとに見る(関門は開始時刻が要り、起動直後の新しい方はまだ関門を持たないため)
pub fn wait_pid_exit(pid: u32, timeout_ms: u32) {
    let start = crate::util::tick_ms();
    while running_pids().contains(&pid) && crate::util::tick_ms() - start < timeout_ms as u64 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_token_reads() {
        let t = self_token().expect("token");
        assert!(!current_user().unwrap_or_default().is_empty());
        let rid = token_integrity_rid(&t).unwrap();
        assert!(rid >= 0x1000, "rid {rid:#x}");
        assert_eq!(integrity_label(0x2000), "中");
        assert_eq!(integrity_label(0x3000), "高");
        assert_eq!(integrity_label(0x4000), "システム");
        assert_eq!(integrity_label(0x1000), "低");
        let _ = is_elevated();
        let p = self_final_path();
        assert!(p.to_lowercase().ends_with(".exe") && !p.starts_with("\\\\?\\"), "{p}");
        assert_eq!(strip_verbatim("\\\\?\\UNC\\srv\\s\\a.exe"), "\\\\srv\\s\\a.exe");
        assert_eq!(restart_args(42), "--after-restart 42");
    }

    #[test]
    fn wait_returns_for_gone_and_times_out_for_self() {
        let me = crate::util::self_pid();
        assert!(running_pids().contains(&me));
        let t = crate::util::tick_ms();
        wait_pid_exit(me, 300);
        assert!(crate::util::tick_ms() - t >= 250);
        let t = crate::util::tick_ms();
        wait_pid_exit(u32::MAX - 3, 5000); // 無い PID はすぐ返る
        assert!(crate::util::tick_ms() - t < 1000);
    }
}
