//! スタートアップの一覧と有効・無効(担当 B。FR-38〜FR-40・D-26)。書くのは StartupApproved の値だけ(INV-13)。
//! レジストリの読み書きの小道具(taskmgr_replace.rs と共有)もここに置く。

use crate::details::SignatureCache;
use crate::jobs::{OpError, SigStatus};
use crate::util::{from_wide, pcwstr, wide};
use procsight_core::startup::{Approved, StartupItem, StartupLocation, decode_approved, encode_approved, exe_from_command, file_name};
use std::collections::HashMap;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_BINARY, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteValueW,
    RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, PWSTR};

// ---- レジストリの小道具 ----

pub(crate) const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;

pub(crate) struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: RegOpenKeyExW / RegCreateKeyExW が返した有効なキーで、1 回だけ閉じる
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

pub(crate) fn open_key(root: HKEY, path: &str, sam: REG_SAM_FLAGS) -> Result<Key, u32> {
    let p = wide(path);
    let mut k = HKEY::default();
    // SAFETY: p は 0 で終わる。k はこの関数の中の変数
    let rc = unsafe { RegOpenKeyExW(root, pcwstr(&p), None, sam, &mut k) };
    if rc.0 != 0 { Err(rc.0) } else { Ok(Key(k)) }
}

/// キーが無ければ作って開く(値を書くときだけ)
pub(crate) fn create_key(root: HKEY, path: &str, sam: REG_SAM_FLAGS) -> Result<Key, u32> {
    let p = wide(path);
    let mut k = HKEY::default();
    // SAFETY: p は 0 で終わる。k はこの関数の中の変数
    let rc = unsafe {
        RegCreateKeyExW(root, pcwstr(&p), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, sam, None, &mut k, None)
    };
    if rc.0 != 0 { Err(rc.0) } else { Ok(Key(k)) }
}

/// 値を読む。無ければ Ok(None)
pub(crate) fn read_value(k: &Key, name: &str) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>, u32> {
    let n = wide(name);
    let mut buf = vec![0u8; 512];
    loop {
        let (mut ty, mut len) = (REG_VALUE_TYPE::default(), buf.len() as u32);
        // SAFETY: n は 0 で終わる。buf の長さを len で渡し、書かれた len バイトだけを読む
        let rc = unsafe { RegQueryValueExW(k.0, pcwstr(&n), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)) };
        match rc.0 {
            0 => {
                buf.truncate(len as usize);
                return Ok(Some((ty, buf)));
            }
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_MORE_DATA => buf = vec![0u8; len as usize + 2],
            c => return Err(c),
        }
    }
}

pub(crate) fn write_value(k: &Key, name: &str, ty: REG_VALUE_TYPE, data: &[u8]) -> Result<(), u32> {
    let n = wide(name);
    // SAFETY: n は 0 で終わる。data はこの呼び出しの間生きている
    let rc = unsafe { RegSetValueExW(k.0, pcwstr(&n), None, ty, Some(data)) };
    if rc.0 != 0 { Err(rc.0) } else { Ok(()) }
}

/// 値を消す。無ければ何もしない
pub(crate) fn delete_value(k: &Key, name: &str) -> Result<(), u32> {
    let n = wide(name);
    // SAFETY: n は 0 で終わる
    let rc = unsafe { RegDeleteValueW(k.0, pcwstr(&n)) };
    if rc.0 != 0 && rc.0 != ERROR_FILE_NOT_FOUND { Err(rc.0) } else { Ok(()) }
}

/// REG_SZ のバイト列を文字列に(末尾の 0 は除く)
pub(crate) fn reg_string(data: &[u8]) -> String {
    let u: Vec<u16> = data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    from_wide(&u)
}

/// 文字列を REG_SZ のバイト列に(末尾の 0 を含む)
pub(crate) fn reg_string_bytes(s: &str) -> Vec<u8> {
    wide(s).iter().flat_map(|c| c.to_le_bytes()).collect()
}

/// キーの値を全部 (名前, 種類, 中身) で
fn enum_values(k: &Key) -> Vec<(String, REG_VALUE_TYPE, Vec<u8>)> {
    let mut out = Vec::new();
    let mut name = vec![0u16; 16384];
    let mut data = vec![0u8; 4096];
    let mut i = 0u32;
    loop {
        let (mut nlen, mut dlen, mut ty) = (name.len() as u32, data.len() as u32, 0u32);
        // SAFETY: name・data の長さをそれぞれ渡し、書かれた分だけを読む
        let rc = unsafe {
            RegEnumValueW(
                k.0,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                &mut nlen,
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut dlen),
            )
        };
        match rc.0 {
            0 => {
                out.push((
                    String::from_utf16_lossy(&name[..nlen as usize]),
                    REG_VALUE_TYPE(ty),
                    data[..dlen as usize].to_vec(),
                ));
                i += 1;
            }
            ERROR_MORE_DATA => data = vec![0u8; dlen as usize + 2],
            _ => return out, // ERROR_NO_MORE_ITEMS など
        }
    }
}

// ---- スタートアップ(D-26) ----

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN32: &str = "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved";

/// その場所の有効・無効を覚えるキー(根、パス)。タスクとストア アプリは StartupApproved を使わないので None
fn approved_key(loc: StartupLocation) -> Option<(HKEY, String)> {
    let (root, sub) = match loc {
        StartupLocation::RegistryUser => (HKEY_CURRENT_USER, "Run"),
        StartupLocation::RegistryMachine => (HKEY_LOCAL_MACHINE, "Run"),
        StartupLocation::Registry32 => (HKEY_LOCAL_MACHINE, "Run32"),
        StartupLocation::FolderUser => (HKEY_CURRENT_USER, "StartupFolder"),
        StartupLocation::FolderCommon => (HKEY_LOCAL_MACHINE, "StartupFolder"),
        StartupLocation::ScheduledTask | StartupLocation::StoreApp => return None,
    };
    Some((root, format!("{APPROVED}\\{sub}")))
}

fn approved_values(loc: StartupLocation) -> HashMap<String, Vec<u8>> {
    let Some((root, path)) = approved_key(loc) else { return HashMap::new() };
    let Ok(k) = open_key(root, &path, KEY_READ | KEY_WOW64_64KEY) else { return HashMap::new() };
    enum_values(&k).into_iter().map(|(n, _, d)| (n.to_lowercase(), d)).collect()
}

/// パスの無い名前(rundll32 など)は System32 の中を探す
fn locate(exe: &str) -> String {
    if exe.contains('\\') {
        return exe.to_string();
    }
    let base = format!("{}\\System32\\{exe}", crate::util::system_root());
    for cand in [base.clone(), format!("{base}.exe")] {
        if std::path::Path::new(&cand).is_file() {
            return cand;
        }
    }
    exe.to_string()
}

fn make_item(
    name: String,
    location: StartupLocation,
    command: String,
    exe_path: Option<String>,
    approved: Option<&[u8]>,
    sig: &SignatureCache,
    touch_free_names: &[String],
) -> StartupItem {
    let exe_exists = exe_path.as_deref().is_some_and(|p| std::path::Path::new(p).is_file());
    let touch_free = exe_path
        .as_deref()
        .is_some_and(|p| touch_free_names.iter().any(|t| file_name(p).eq_ignore_ascii_case(t)));
    // 触れない名前と、exe が分からない物は署名を確かめない(FR-38・INV-1)
    let publisher = match &exe_path {
        Some(p) if exe_exists && !touch_free => {
            let s = sig.check(p);
            if matches!(s.status, SigStatus::Embedded | SigStatus::Catalog) { s.signer } else { None }
        }
        _ => None,
    };
    StartupItem { name, location, command, exe_path, exe_exists, approved: decode_approved(approved), publisher, readonly_reason: None }
}

/// .lnk の指す先と引数(IShellLinkW。指す先は解決しない)
fn resolve_lnk(path: &str) -> Option<(String, String)> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::Interface;
    let _com = crate::shell::Com::init();
    let p = wide(path);
    // SAFETY: COM はこのスレッドで初期化した。p と buf はこの関数の中の配列
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(pcwstr(&p), STGM_READ).ok()?;
        let mut target = vec![0u16; 1024];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
        let mut args = vec![0u16; 4096];
        let _ = link.GetArguments(&mut args);
        let t = from_wide(&target);
        (!t.is_empty()).then(|| (t, from_wide(&args)))
    }
}

fn known_folder(id: &windows::core::GUID) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: 返った文字列を読んでから CoTaskMemFree で返す
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = crate::util::pwstr_to_string(p);
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        Some(s)
    }
}

/// `touch_free_names` に一致する exe は署名を確かめない(FR-38・INV-1)
pub fn load(sig: &SignatureCache, touch_free_names: &[String]) -> Result<Vec<StartupItem>, OpError> {
    use windows::Win32::UI::Shell::{FOLDERID_CommonStartup, FOLDERID_Startup};
    let mut items = Vec::new();
    for (root, path, loc) in [
        (HKEY_CURRENT_USER, RUN, StartupLocation::RegistryUser),
        (HKEY_LOCAL_MACHINE, RUN, StartupLocation::RegistryMachine),
        (HKEY_LOCAL_MACHINE, RUN32, StartupLocation::Registry32),
    ] {
        let Ok(k) = open_key(root, path, KEY_READ | KEY_WOW64_64KEY) else { continue };
        let approved = approved_values(loc);
        for (name, ty, data) in enum_values(&k) {
            if name.is_empty() || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
                continue;
            }
            let command = reg_string(&data);
            let exe = exe_from_command(&command, &crate::util::expand_env).map(|e| locate(&e));
            let a = approved.get(&name.to_lowercase()).map(|v| v.as_slice());
            items.push(make_item(name, loc, command, exe, a, sig, touch_free_names));
        }
    }
    for (id, loc) in [(FOLDERID_Startup, StartupLocation::FolderUser), (FOLDERID_CommonStartup, StartupLocation::FolderCommon)] {
        let Some(dir) = known_folder(&id) else { continue };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let approved = approved_values(loc);
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !e.file_type().is_ok_and(|t| t.is_file()) || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let full = e.path().to_string_lossy().into_owned();
            let (command, exe) = if name.to_lowercase().ends_with(".lnk") {
                match resolve_lnk(&full) {
                    Some((t, a)) if a.is_empty() => (format!("\"{t}\""), Some(t)),
                    Some((t, a)) => (format!("\"{t}\" {a}"), Some(t)),
                    None => (full.clone(), None),
                }
            } else {
                (format!("\"{full}\""), Some(full.clone()))
            };
            let a = approved.get(&name.to_lowercase()).map(|v| v.as_slice());
            items.push(make_item(name, loc, command, exe, a, sig, touch_free_names));
        }
    }
    // v0.3.0: タスク スケジューラ(ログオン時・起動時)とストア アプリ(読むだけ)
    items.extend(crate::startup_tasks::list(sig, touch_free_names));
    items.extend(crate::startup_store::list());
    Ok(items)
}

pub fn set(name: &str, location: StartupLocation, enabled: bool) -> Result<(), OpError> {
    use windows::Win32::System::Registry::{KEY_QUERY_VALUE, KEY_SET_VALUE};
    let (root, path) = match location {
        StartupLocation::ScheduledTask => return crate::startup_tasks::set(name, enabled),
        StartupLocation::StoreApp => return crate::startup_store::set(name, enabled),
        _ => approved_key(location).ok_or(OpError::Blocked("この項目は切り替えられません"))?,
    };
    let k = create_key(root, &path, KEY_SET_VALUE | KEY_QUERY_VALUE | KEY_WOW64_64KEY).map_err(OpError::from_code)?;
    // E-30: 形の分からない値は書き換えない
    let current = read_value(&k, name).map_err(OpError::from_code)?;
    if decode_approved(current.as_ref().map(|(_, d)| d.as_slice())) == Approved::Unknown {
        return Err(OpError::Blocked("この項目の状態が分からないため、切り替えられません"));
    }
    write_value(&k, name, REG_BINARY, &encode_approved(enabled, crate::util::now_filetime())).map_err(OpError::from_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_keys_match_task_manager() {
        let k = |l| approved_key(l).unwrap();
        assert_eq!(k(StartupLocation::RegistryUser).1, format!("{APPROVED}\\Run"));
        assert_eq!(k(StartupLocation::Registry32).1, format!("{APPROVED}\\Run32"));
        assert_eq!(k(StartupLocation::FolderCommon).1, format!("{APPROVED}\\StartupFolder"));
        assert_eq!(k(StartupLocation::FolderUser).0, HKEY_CURRENT_USER);
        assert_eq!(k(StartupLocation::RegistryMachine).0, HKEY_LOCAL_MACHINE);
        assert!(approved_key(StartupLocation::ScheduledTask).is_none() && approved_key(StartupLocation::StoreApp).is_none());
        assert_eq!(reg_string(&reg_string_bytes("a\\b")), "a\\b");
        assert!(locate("cmd").to_lowercase().ends_with("\\system32\\cmd.exe"));
    }

    #[test]
    fn list_reads() {
        // 名前は出さず、件数だけを出す
        let sig = SignatureCache::new();
        let items = load(&sig, &["csrss.exe".into()]).unwrap();
        let count = |l: StartupLocation| items.iter().filter(|i| i.location == l).count();
        println!(
            "スタートアップ: 全 {} 件 (ユーザー {}、全ユーザー {}、32 ビット {}、フォルダ {}・{})、発行元あり {}、不明 {}",
            items.len(),
            count(StartupLocation::RegistryUser),
            count(StartupLocation::RegistryMachine),
            count(StartupLocation::Registry32),
            count(StartupLocation::FolderUser),
            count(StartupLocation::FolderCommon),
            items.iter().filter(|i| i.publisher.is_some()).count(),
            items.iter().filter(|i| i.approved == Approved::Unknown).count(),
        );
        for (label, loc) in [("Run", StartupLocation::RegistryUser), ("Run32", StartupLocation::Registry32), ("StartupFolder", StartupLocation::FolderUser)] {
            println!("StartupApproved\\{label}(HKCU/HKLM の該当): 値 {} 個", approved_values(loc).len());
        }
    }
}
