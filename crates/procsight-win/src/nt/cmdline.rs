//! コマンドライン(担当 B。D-19・事実23)。NtQueryInformationProcess の情報の種類 60(ProcessCommandLineInformation)。
//! 結果は UNICODE_STRING とその後ろの文字列。長さが足りなければ返った長さで取り直す。

use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows::Win32::Foundation::{HANDLE, UNICODE_STRING};

const STATUS_BUFFER_OVERFLOW: i32 = 0x8000_0005u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;

/// `process` は PROCESS_QUERY_LIMITED_INFORMATION 以上で開いたハンドル(関門の物か自分の擬似ハンドル)。
/// 失敗したら NTSTATUS
pub fn query(process: HANDLE) -> Result<String, i32> {
    query_with(process, 4096)
}

/// `first_bytes` は最初に用意するバッファの大きさ(足りなければ返った長さで取り直す)
fn query_with(process: HANDLE, first_bytes: usize) -> Result<String, i32> {
    // UNICODE_STRING の並び(8 バイト)に合わせるため u64 で持つ
    let mut buf = vec![0u64; first_bytes.div_ceil(8).max(1)];
    for _ in 0..4 {
        let mut ret = 0u32;
        let len = (buf.len() * 8) as u32;
        // SAFETY: buf の長さ(バイト)を渡し、書かれるのはその中だけ
        let st = unsafe {
            NtQueryInformationProcess(process, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), len, &mut ret)
        };
        match st.0 {
            s if s >= 0 => {
                // SAFETY: 成功したので先頭は UNICODE_STRING で、Buffer は buf の中(後ろ)を Length バイト指す
                let s = unsafe {
                    let us = &*(buf.as_ptr() as *const UNICODE_STRING);
                    if us.Buffer.is_null() || us.Length == 0 {
                        return Ok(String::new());
                    }
                    let base = buf.as_ptr() as usize;
                    let p = us.Buffer.0 as usize;
                    let n = us.Length as usize;
                    if p < base || p + n > base + buf.len() * 8 {
                        return Err(STATUS_BUFFER_OVERFLOW); // バッファの外を指すなら使わない
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(us.Buffer.0, n / 2))
                };
                return Ok(s);
            }
            super::STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_OVERFLOW | STATUS_BUFFER_TOO_SMALL => {
                let want = (ret as usize).max(buf.len() * 16);
                buf = vec![0u64; want.div_ceil(8)];
            }
            s => return Err(s),
        }
    }
    Err(super::STATUS_INFO_LENGTH_MISMATCH)
}

/// 自分のプロセスで取れるか(画面設計書 §7.4 の ApiAvailability::cmdline に使う)
pub fn self_check() -> bool {
    // SAFETY: 自分の擬似ハンドル(閉じなくてよい)
    let me = unsafe { windows::Win32::System::Threading::GetCurrentProcess() };
    query(me).is_ok_and(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_command_line() {
        // SAFETY: 自分の擬似ハンドル
        let me = unsafe { windows::Win32::System::Threading::GetCurrentProcess() };
        let s = query(me).unwrap();
        let exe = std::env::current_exe().unwrap();
        let stem = exe.file_stem().unwrap().to_string_lossy().to_lowercase();
        assert!(!s.is_empty());
        assert!(s.to_lowercase().contains(&stem), "コマンドラインに exe 名が無い");
        assert!(self_check());
        // 小さいバッファから始めても、取り直して同じ物が返る
        assert_eq!(query_with(me, 16).unwrap(), s);
        assert_eq!(query_with(me, 0).unwrap(), s);
    }
}
