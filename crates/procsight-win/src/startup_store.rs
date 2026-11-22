//! ストア アプリのスタートアップ(StartupTask。v0.3.0)。HKCU の SystemAppData の下の、パッケージごと・タスクごとの
//! State と LastDisabledTime を読むだけ。State の意味の解釈は純粋な関数 `decode_state` に置く。
//! 切り替えは、前後の実物を取るまで作らない(書く処理は入れない。`set` はいつも断る)。

use crate::jobs::OpError;
use crate::startup::{open_key, read_value};
use crate::util::{pcwstr, wide};
use procsight_core::startup::{Approved, StartupItem, StartupLocation};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_QWORD, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
};
use windows::core::PWSTR;

/// 一覧で切り替えのボタンを灰色にする理由
pub const READONLY_REASON: &str = "ストア アプリの切り替えは、Windows の設定の「スタートアップ」で行ってください";

const SYSTEM_APP_DATA: &str =
    "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\SystemAppData";

/// 1601-01-01 から 1970-01-01 までの秒
const UNIX_TO_FILETIME_SECS: u64 = 11_644_473_600;

/// State(DWORD)と LastDisabledTime(QWORD の Unix 秒)の解釈。2 は有効、0・1 は無効、他は不明。
/// 実物(2026-11-22)では、有効(2)の行にも LastDisabledTime が残っていることがある(前に無効にした跡)
pub fn decode_state(state: u32, last_disabled_unix: Option<u64>) -> Approved {
    match state {
        2 => Approved::Enabled,
        0 | 1 => Approved::Disabled(last_disabled_unix.filter(|&s| s != 0).map_or(0, unix_to_filetime)),
        _ => Approved::Unknown,
    }
}

fn unix_to_filetime(secs: u64) -> u64 {
    secs.saturating_add(UNIX_TO_FILETIME_SECS).saturating_mul(10_000_000)
}

/// パッケージ ファミリ名から読みやすい名前(`_` の前。例 `Microsoft.Windows.Photos_8wekyb3d8bbwe` → `Microsoft.Windows.Photos`)
pub fn readable_name(family: &str) -> &str {
    family.split('_').next().filter(|s| !s.is_empty()).unwrap_or(family)
}

/// 開いて閉じるだけのキー(サブキーの列挙に生の HKEY が要るため)
struct RawKey(HKEY);

impl Drop for RawKey {
    fn drop(&mut self) {
        // SAFETY: RegOpenKeyExW が返した有効なキーで、1 回だけ閉じる
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// HKCU\path の直下のサブキーの名前。開けなければ空
fn subkeys(path: &str) -> Vec<String> {
    let p = wide(path);
    let mut k = HKEY::default();
    // SAFETY: p は 0 で終わる。k はこの関数の中の変数
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, pcwstr(&p), None, KEY_READ, &mut k) }.0 != 0 {
        return Vec::new();
    }
    let k = RawKey(k);
    let mut out = Vec::new();
    let mut name = vec![0u16; 256]; // キーの名前は 255 文字まで
    for i in 0u32.. {
        let mut len = name.len() as u32;
        // SAFETY: name の長さを len で渡し、書かれた len 文字だけを読む
        let rc = unsafe { RegEnumKeyExW(k.0, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if rc.0 != 0 {
            break; // ERROR_NO_MORE_ITEMS など
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

/// タスクのキーの State と LastDisabledTime。State が DWORD で無ければ(タスクでないキー)None
fn read_task(path: &str) -> Option<(u32, Option<u64>)> {
    let k = open_key(HKEY_CURRENT_USER, path, KEY_READ).ok()?;
    let (ty, data) = read_value(&k, "State").ok()??;
    if ty != REG_DWORD || data.len() < 4 {
        return None;
    }
    let state = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let last = match read_value(&k, "LastDisabledTime") {
        Ok(Some((t, d))) if t == REG_QWORD && d.len() >= 8 => d[..8].try_into().ok().map(u64::from_le_bytes),
        _ => None,
    };
    Some((state, last))
}

/// `unique` が false なら(`_` の前が同じ別のパッケージがある)、ファミリ名の全体を名前にする
fn make_item(family: &str, task: &str, several: bool, unique: bool, state: u32, last: Option<u64>) -> StartupItem {
    let base = if unique { readable_name(family) } else { family };
    StartupItem {
        // 同じパッケージにタスクが 2 つ以上あれば、選択の鍵(名前と場所)が重ならないようタスク ID を足す
        name: if several { format!("{base} ({task})") } else { base.to_string() },
        location: StartupLocation::StoreApp,
        command: format!("{family}!{task}"),
        exe_path: None,
        exe_exists: true,
        approved: decode_state(state, last),
        publisher: None,
        readonly_reason: Some(READONLY_REASON),
    }
}

/// ストア アプリのスタートアップの一覧(読むだけ)。State を持つキーだけをタスクとみなす
pub fn list() -> Vec<StartupItem> {
    /// (タスク ID, State, LastDisabledTime)
    type Task = (String, u32, Option<u64>);
    let found: Vec<(String, Vec<Task>)> = subkeys(SYSTEM_APP_DATA)
        .into_iter()
        .map(|family| {
            let fam_path = format!("{SYSTEM_APP_DATA}\\{family}");
            let tasks = subkeys(&fam_path)
                .into_iter()
                .filter_map(|t| read_task(&format!("{fam_path}\\{t}")).map(|(s, l)| (t, s, l)))
                .collect::<Vec<_>>();
            (family, tasks)
        })
        .filter(|(_, tasks)| !tasks.is_empty())
        .collect();
    let mut items = Vec::new();
    for (family, tasks) in &found {
        let unique = found.iter().filter(|(f, _)| readable_name(f) == readable_name(family)).count() == 1;
        let several = tasks.len() > 1;
        items.extend(tasks.iter().map(|(t, s, l)| make_item(family, t, several, unique, *s, *l)));
    }
    items
}

/// 書かない。画面からは灰色のボタンで止めるが、Job が来ても断る
pub fn set(_name: &str, _enabled: bool) -> Result<(), OpError> {
    Err(OpError::Blocked(READONLY_REASON))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_values_from_the_real_pc() {
        // 2026-11-22 にこの PC で読んだ実物の組(StoreStartupTask-2026-11-22.md)
        assert_eq!(decode_state(2, None), Approved::Enabled);
        assert_eq!(decode_state(2, Some(1_661_661_351)), Approved::Enabled, "有効に戻した跡が残っていても有効");
        assert_eq!(decode_state(0, None), Approved::Disabled(0));
        assert_eq!(decode_state(0, Some(0)), Approved::Disabled(0));
        let Approved::Disabled(ft) = decode_state(1, Some(1_665_386_436)) else { panic!() };
        // 1665386436 = 2022-10-10 07:20:36 UTC
        assert_eq!(ft, (1_665_386_436 + 11_644_473_600) * 10_000_000);
        assert_eq!(&procsight_core::format::filetime(ft, 0)[..16], "2022-10-10 07:20");
        assert!(matches!(decode_state(0, Some(1_661_661_351)), Approved::Disabled(f) if f != 0));
        assert_eq!(decode_state(3, None), Approved::Unknown);
        assert_eq!(decode_state(4, Some(1)), Approved::Unknown);
        assert_eq!(unix_to_filetime(u64::MAX), u64::MAX, "あふれない");
    }

    #[test]
    fn names_and_items() {
        assert_eq!(readable_name("Microsoft.Windows.Photos_8wekyb3d8bbwe"), "Microsoft.Windows.Photos");
        assert_eq!(readable_name("NoUnderscore"), "NoUnderscore");
        assert_eq!(readable_name("_odd"), "_odd");
        let it = make_item("Example.App_abc", "Start", false, true, 2, None);
        assert_eq!((it.name.as_str(), it.command.as_str()), ("Example.App", "Example.App_abc!Start"));
        assert_eq!(it.location, StartupLocation::StoreApp);
        assert_eq!(it.readonly_reason, Some(READONLY_REASON));
        assert!(it.exe_exists && it.exe_path.is_none() && it.publisher.is_none());
        assert_eq!(make_item("Example.App_abc", "Start", true, true, 0, None).name, "Example.App (Start)");
        assert_eq!(make_item("Example.App_abc", "Start", false, false, 0, None).name, "Example.App_abc", "別の発行元と重なる時");
        assert!(set("x", true).is_err());
    }

    #[test]
    fn list_reads() {
        // 読むだけ。名前は出さず件数だけ
        let items = list();
        let on = items.iter().filter(|i| i.approved == Approved::Enabled).count();
        let unknown = items.iter().filter(|i| i.approved == Approved::Unknown).count();
        println!("ストア アプリ: 全 {} 件 (有効 {on}、無効 {}、不明 {unknown})", items.len(), items.len() - on - unknown);
        assert!(items.iter().all(|i| i.location == StartupLocation::StoreApp && i.readonly_reason.is_some()));
        let mut keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), items.len(), "名前が重ならない");
    }
}
