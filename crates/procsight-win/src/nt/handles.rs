//! ハンドルの一覧(担当 B。D-22・事実22)。SystemExtendedHandleInformation(64)と、NtQueryObject の種類の表・名前。
//! 構造体はバイトの位置で読む。種類の表は最初に使うときに 1 回作り、自分で作った Event で位置を確かめてから使う。

use std::collections::HashMap;
use std::sync::OnceLock;
use windows::Wdk::Foundation::{NtQueryObject, OBJECT_INFORMATION_CLASS, ObjectTypeInformation};
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
use windows::Win32::Foundation::{CloseHandle, HANDLE, UNICODE_STRING};

const SYSTEM_EXTENDED_HANDLE_INFORMATION: SYSTEM_INFORMATION_CLASS = SYSTEM_INFORMATION_CLASS(64);
const OBJECT_NAME_INFORMATION: OBJECT_INFORMATION_CLASS = OBJECT_INFORMATION_CLASS(1);
const OBJECT_TYPES_INFORMATION: OBJECT_INFORMATION_CLASS = OBJECT_INFORMATION_CLASS(3);
const STATUS_BUFFER_OVERFLOW: i32 = 0x8000_0005u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;

/// SYSTEM_HANDLE_INFORMATION_EX の先頭(NumberOfHandles usize、Reserved usize)
const HANDLES_HEADER: usize = 16;
/// SYSTEM_HANDLE_TABLE_ENTRY_INFO_EX の 1 件(x64): 0x00 Object ptr、0x08 UniqueProcessId usize、0x10 HandleValue usize、
/// 0x18 GrantedAccess u32、0x1C CreatorBackTraceIndex u16、0x1E ObjectTypeIndex u16、0x20 HandleAttributes u32、0x24 Reserved u32
const HANDLE_ENTRY: usize = 40;

/// OBJECT_TYPE_INFORMATION の 1 項目の大きさ。windows crate の PUBLIC_OBJECT_TYPE_INFORMATION
/// (winternl.h の文書の形: UNICODE_STRING TypeName と ULONG Reserved[22])と同じ 0x68 バイト。
/// 実行時にも、各項目の TypeName.Buffer が「項目の先頭 + 0x68」を指すことを確かめる
const TYPE_ENTRY: usize = 0x68;
/// TypeIndex(UCHAR)の位置。文書の Reserved の中(非公開。phnt の OBJECT_TYPE_INFORMATION では
/// GenericMapping 0x44・ValidAccessMask 0x54・SecurityRequired 0x58・MaintainHandleCount 0x59 の次)。
/// 自分で作った Event のハンドルの ObjectTypeIndex がこの表で「Event」になることを確かめてから使う
const TYPE_INDEX_AT: usize = 0x5A;
const _: () = assert!(size_of::<windows::Win32::System::WindowsProgramming::PUBLIC_OBJECT_TYPE_INFORMATION>() == TYPE_ENTRY);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawHandle {
    pub pid: u32,
    pub value: usize,
    pub access: u32,
    pub type_index: u16,
}

fn rd_usize(b: &[u8], at: usize) -> usize {
    usize::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}
fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or_default())
}
fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap_or_default())
}

/// 全プロセスのハンドルを取り、`pid` の物だけを返す。失敗したら NTSTATUS
pub fn for_pid(pid: u32) -> Result<Vec<RawHandle>, i32> {
    let mut buf = vec![0u64; 1 << 17]; // 1 MiB から
    loop {
        let mut ret = 0u32;
        let len = (buf.len() * 8) as u32;
        // SAFETY: buf の長さ(バイト)を渡し、書かれるのはその中だけ
        let st = unsafe {
            NtQuerySystemInformation(SYSTEM_EXTENDED_HANDLE_INFORMATION, buf.as_mut_ptr().cast(), len, &mut ret)
        };
        if st.0 == super::STATUS_INFO_LENGTH_MISMATCH {
            if buf.len() >= 1 << 26 {
                return Err(st.0); // 512 MiB を超えるなら諦める
            }
            let want = (ret as usize + (1 << 20)).max(buf.len() * 16);
            buf = vec![0u64; want.div_ceil(8)];
            continue;
        }
        if st.0 < 0 {
            return Err(st.0);
        }
        // SAFETY: buf は u64 の配列で、その全体を u8 として読むだけ
        let b = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
        return Ok(parse_handles(b, pid));
    }
}

fn parse_handles(b: &[u8], pid: u32) -> Vec<RawHandle> {
    let n = rd_usize(b, 0);
    let fit = (b.len() - HANDLES_HEADER) / HANDLE_ENTRY;
    (0..n.min(fit))
        .map(|i| HANDLES_HEADER + i * HANDLE_ENTRY)
        .filter(|&at| rd_usize(b, at + 0x08) == pid as usize)
        .map(|at| RawHandle {
            pid,
            value: rd_usize(b, at + 0x10),
            access: rd_u32(b, at + 0x18),
            type_index: rd_u16(b, at + 0x1E),
        })
        .collect()
}

/// NtQueryObject の結果が UNICODE_STRING で始まる物(名前・種類)を取る。失敗したら NTSTATUS
fn query_unicode(h: HANDLE, class: OBJECT_INFORMATION_CLASS) -> Result<Option<String>, i32> {
    let mut buf = vec![0u64; 128];
    for _ in 0..4 {
        let mut ret = 0u32;
        let len = (buf.len() * 8) as u32;
        // SAFETY: buf の長さ(バイト)を渡し、書かれるのはその中だけ。h は呼ぶ側が持つ有効なハンドル
        let st = unsafe { NtQueryObject(Some(h), class, Some(buf.as_mut_ptr().cast()), len, Some(&mut ret)) };
        match st.0 {
            s if s >= 0 => return Ok(unicode_at(&buf, 0)),
            super::STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_OVERFLOW | STATUS_BUFFER_TOO_SMALL => {
                buf = vec![0u64; (ret as usize).max(buf.len() * 16).div_ceil(8)];
            }
            s => return Err(s),
        }
    }
    Err(super::STATUS_INFO_LENGTH_MISMATCH)
}

/// buf の at バイト目の UNICODE_STRING を読む。Buffer が buf の外を指していれば None
fn unicode_at(buf: &[u64], at: usize) -> Option<String> {
    let base = buf.as_ptr() as usize;
    let end = base + buf.len() * 8;
    if at + size_of::<UNICODE_STRING>() > buf.len() * 8 {
        return None;
    }
    // SAFETY: at は buf の中で、UNICODE_STRING の大きさが収まることを上で確かめた。読み取りは read_unaligned
    let us = unsafe { std::ptr::read_unaligned((base + at) as *const UNICODE_STRING) };
    let p = us.Buffer.0 as usize;
    let n = us.Length as usize;
    if us.Buffer.is_null() || n == 0 || p < base || p + n > end {
        return None;
    }
    // SAFETY: [p, p+n) が buf の中にあることを上で確かめた
    Some(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p as *const u16, n / 2) }))
}

/// ハンドルの種類の名前(ObjectTypeInformation。文書にある情報の種類 2)。名前の問い合わせと違い止まらない
pub fn query_type(h: HANDLE) -> Option<String> {
    query_unicode(h, ObjectTypeInformation).ok().flatten()
}

/// ハンドルの名前(ObjectNameInformation)。名前が無ければ Ok(None)。止まることがあるので HandleNameWorker から呼ぶ
pub fn query_name(h: HANDLE) -> Result<Option<String>, i32> {
    query_unicode(h, OBJECT_NAME_INFORMATION)
}

/// 種類の表を作る(ObjectTypesInformation)。並びが想定と違えば None
fn build_types() -> Option<HashMap<u16, String>> {
    let mut buf = vec![0u64; 4096];
    let mut ok = false;
    for _ in 0..4 {
        let mut ret = 0u32;
        let len = (buf.len() * 8) as u32;
        // SAFETY: buf の長さ(バイト)を渡し、書かれるのはその中だけ。ハンドルは渡さない(None)
        let st = unsafe { NtQueryObject(None, OBJECT_TYPES_INFORMATION, Some(buf.as_mut_ptr().cast()), len, Some(&mut ret)) };
        if st.0 >= 0 {
            ok = true;
            break;
        }
        if st.0 != super::STATUS_INFO_LENGTH_MISMATCH {
            return None;
        }
        buf = vec![0u64; (ret as usize).max(buf.len() * 2).div_ceil(8)];
    }
    if !ok {
        return None;
    }
    parse_types(&buf)
}

/// OBJECT_TYPES_INFORMATION: 先頭は ULONG NumberOfTypes、最初の項目は 8 バイトに切り上げた位置(ALIGN_UP(4, ULONG_PTR))。
/// 次の項目は「項目 + 0x68 + TypeName.MaximumLength」を 8 に切り上げた位置
fn parse_types(buf: &[u64]) -> Option<HashMap<u16, String>> {
    let base = buf.as_ptr() as usize;
    let total = buf.len() * 8;
    // SAFETY: buf は u64 の配列で、その全体を u8 として読むだけ
    let b = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, total) };
    let count = rd_u32(b, 0) as usize;
    if count == 0 || count > 1000 {
        return None;
    }
    let mut at = 8usize;
    let mut map = HashMap::new();
    for _ in 0..count {
        if at + TYPE_ENTRY > total {
            return None;
        }
        let max_len = rd_u16(b, at + 2) as usize;
        let buffer_ptr = rd_usize(b, at + 8);
        if buffer_ptr != base + at + TYPE_ENTRY {
            return None; // 項目の大きさが 0x68 でない
        }
        let name = unicode_at(buf, at)?;
        let index = b[at + TYPE_INDEX_AT] as u16;
        map.insert(index, name);
        at = (at + TYPE_ENTRY + max_len).div_ceil(8) * 8;
    }
    // 自分で作った Event のハンドルで、TypeIndex の位置を確かめる
    verify_with_event(&map).then_some(map)
}

fn verify_with_event(map: &HashMap<u16, String>) -> bool {
    use windows::Win32::System::Threading::CreateEventW;
    // SAFETY: 名前なしのイベントを作るだけ。最後に閉じる
    let Ok(ev) = (unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }) else { return false };
    let by_api = query_unicode(ev, ObjectTypeInformation).ok().flatten();
    let by_table = for_pid(std::process::id())
        .ok()
        .and_then(|v| v.into_iter().find(|h| h.value == ev.0 as usize))
        .and_then(|h| map.get(&h.type_index).cloned());
    // SAFETY: 上で作ったハンドルを 1 回だけ閉じる
    unsafe {
        let _ = CloseHandle(ev);
    }
    by_api.as_deref() == Some("Event") && by_table.as_deref() == Some("Event")
}

/// 種類の番号 → 名前の表。最初に呼んだときに 1 回作る。作れなければ None(D-6)
pub fn type_names() -> Option<&'static HashMap<u16, String>> {
    static TYPES: OnceLock<Option<HashMap<u16, String>>> = OnceLock::new();
    TYPES.get_or_init(build_types).as_ref()
}

/// 種類の表が作れて、自分のハンドルが読めるか(ApiAvailability::handles に使う)
pub fn self_check() -> bool {
    type_names().is_some() && for_pid(std::process::id()).is_ok_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_table_and_own_handles() {
        let t = type_names().expect("種類の表");
        let names: Vec<&str> = t.values().map(|s| s.as_str()).collect();
        for want in ["Process", "Thread", "Event", "File", "Key"] {
            assert!(names.contains(&want), "{want} が無い");
        }
        let mine = for_pid(std::process::id()).unwrap();
        assert!(mine.len() > 5);
        assert!(mine.iter().all(|h| t.contains_key(&h.type_index)), "表に無い種類の番号がある");
        assert!(self_check());
    }

    #[test]
    fn parse_handle_entries() {
        let mut b = vec![0u8; HANDLES_HEADER + 2 * HANDLE_ENTRY];
        b[0] = 2;
        let e1 = HANDLES_HEADER;
        b[e1 + 0x08] = 7;
        b[e1 + 0x10] = 0x44;
        b[e1 + 0x18..e1 + 0x1C].copy_from_slice(&0x1F0003u32.to_le_bytes());
        b[e1 + 0x1E] = 16;
        let e2 = e1 + HANDLE_ENTRY;
        b[e2 + 0x08] = 8;
        let v = parse_handles(&b, 7);
        assert_eq!(v, vec![RawHandle { pid: 7, value: 0x44, access: 0x1F0003, type_index: 16 }]);
    }
}
