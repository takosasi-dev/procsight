//! タスク スケジューラのスタートアップ(ログオン時・起動時のトリガーを持つタスク。v0.3.0)。
//! 読むのも切り替えるのも、公開されている COM の API(ITaskService)だけ。`\Microsoft\` の下は見ない。
//! 切り替えは IRegisteredTask の Enabled を変えるだけ。COM は呼んだスレッド(Worker)で初期化する。

use crate::details::SignatureCache;
use crate::jobs::{OpError, SigStatus};
use crate::shell::Com;
use crate::util::{expand_env, win32_code};
use procsight_core::startup::{Approved, StartupItem, StartupLocation, file_name};
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::TaskScheduler::{
    IExecAction, IRegisteredTask, ITaskFolder, ITaskService, TASK_ACTION_EXEC, TASK_ACTION_TYPE, TASK_ENUM_HIDDEN,
    TASK_TRIGGER_BOOT, TASK_TRIGGER_LOGON, TASK_TRIGGER_TYPE2, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

/// Windows の物が多すぎるので降りないフォルダ(根の直下の `\Microsoft`。`\MicrosoftEdge…` のような名前のタスクは残す)
fn skip_folder(path: &str) -> bool {
    path.trim_end_matches('\\').eq_ignore_ascii_case("\\Microsoft")
}

/// 切り替えてよい名前か(一覧に出す形のパスだけ。`\Microsoft\` の下は一覧に出さないので切り替えもしない)
fn check_name(name: &str) -> Result<(), OpError> {
    let under_ms = name.get(..11).is_some_and(|h| h.eq_ignore_ascii_case("\\Microsoft\\"));
    if !name.starts_with('\\') || name.len() < 2 || under_ms {
        return Err(OpError::Blocked("このタスクは切り替えられません"));
    }
    Ok(())
}

/// 引用符を外す
fn unquote(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix('"').map(|r| r.strip_suffix('"').unwrap_or(r)).unwrap_or(t)
}

/// 一覧に出すコマンド(path に空白があれば引用符で囲み、引数をつなぐ。環境変数は `expand` で展開)
fn command_of(path: &str, args: &str, expand: &dyn Fn(&str) -> String) -> String {
    let p = expand(unquote(path));
    let p = if p.contains(' ') { format!("\"{p}\"") } else { p };
    let a = expand(args.trim());
    if a.is_empty() { p } else { format!("{p} {a}") }
}

/// パスの無い名前(powershell.exe など)は System32 の中を探す(startup.rs の locate と同じ規則)
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

/// 発行元(startup.rs の make_item と同じ規則。触れない名前と exe が無い物は署名を見ない。FR-38・INV-1)
fn publisher(exe: &str, exists: bool, sig: &SignatureCache, touch_free_names: &[String]) -> Option<String> {
    let touch_free = touch_free_names.iter().any(|t| file_name(exe).eq_ignore_ascii_case(t));
    if !exists || touch_free {
        return None;
    }
    let s = sig.check(exe);
    if matches!(s.status, SigStatus::Embedded | SigStatus::Catalog) { s.signer } else { None }
}

fn connect() -> windows::core::Result<ITaskService> {
    // SAFETY: COM は呼んだ側がこのスレッドで初期化した。Connect の引数は空の VARIANT(この PC・今の利用者)
    unsafe {
        let svc: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        let empty = VARIANT::default();
        svc.Connect(&empty, &empty, &empty, &empty)?;
        Ok(svc)
    }
}

/// タスクの (パス, 有効か, 最初の exec の (path, 引数))。ログオン時・起動時のトリガーが無い・exec が無い・読めないなら None
fn read_task(t: &IRegisteredTask) -> Option<(String, bool, String, String)> {
    // SAFETY: COM はこのスレッドで初期化した。out の引数はこの関数の中の変数
    unsafe {
        let def = t.Definition().ok()?;
        let triggers = def.Triggers().ok()?;
        let mut n = 0i32;
        triggers.Count(&mut n).ok()?;
        let at_start = (1..=n).any(|i| {
            let mut ty = TASK_TRIGGER_TYPE2::default();
            triggers.get_Item(i).and_then(|tr| tr.Type(&mut ty)).is_ok() && (ty == TASK_TRIGGER_LOGON || ty == TASK_TRIGGER_BOOT)
        });
        if !at_start {
            return None;
        }
        let actions = def.Actions().ok()?;
        let mut n = 0i32;
        actions.Count(&mut n).ok()?;
        let exec = (1..=n).find_map(|i| {
            let a = actions.get_Item(i).ok()?;
            let mut ty = TASK_ACTION_TYPE::default();
            a.Type(&mut ty).ok()?;
            if ty != TASK_ACTION_EXEC {
                return None;
            }
            let e: IExecAction = a.cast().ok()?;
            let (mut p, mut args) = (BSTR::new(), BSTR::new());
            e.Path(&mut p).ok()?;
            let _ = e.Arguments(&mut args);
            Some((p.to_string(), args.to_string()))
        })?;
        let path = t.Path().ok()?.to_string();
        let enabled = t.Enabled().ok()?.as_bool();
        Some((path, enabled, exec.0, exec.1))
    }
}

fn make_item(path: String, enabled: bool, exe: &str, args: &str, sig: &SignatureCache, touch_free_names: &[String]) -> StartupItem {
    let located = locate(&expand_env(unquote(exe)));
    // パスの無い名前で System32 にも無い物は、作業フォルダや PATH から探されるので、ここでは「ファイルなし」と決めない
    let known = located.contains('\\');
    let exe_exists = !known || std::path::Path::new(&located).is_file();
    StartupItem {
        name: path,
        location: StartupLocation::ScheduledTask,
        command: command_of(exe, args, &expand_env),
        publisher: if known { publisher(&located, exe_exists, sig, touch_free_names) } else { None },
        exe_exists,
        exe_path: known.then_some(located),
        approved: if enabled { Approved::Enabled } else { Approved::Disabled(0) },
        readonly_reason: None,
    }
}

/// ログオン時・起動時に動くタスクの一覧(隠したタスクも含む)。読めないフォルダ・タスクは飛ばす
pub fn list(sig: &SignatureCache, touch_free_names: &[String]) -> Vec<StartupItem> {
    let _com = Com::init();
    let Ok(svc) = connect() else { return Vec::new() };
    // SAFETY: COM はこのスレッドで初期化した
    let Ok(root) = (unsafe { svc.GetFolder(&BSTR::from("\\")) }) else { return Vec::new() };
    let mut items = Vec::new();
    let mut stack: Vec<ITaskFolder> = vec![root];
    while let Some(folder) = stack.pop() {
        // SAFETY: COM はこのスレッドで初期化した。コレクションの添字は 1 から
        unsafe {
            if let Ok(subs) = folder.GetFolders(0) {
                for i in 1..=subs.Count().unwrap_or(0) {
                    let Ok(f) = subs.get_Item(&VARIANT::from(i)) else { continue };
                    if f.Path().is_ok_and(|p| !skip_folder(&p.to_string())) {
                        stack.push(f);
                    }
                }
            }
            let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) else { continue };
            for i in 1..=tasks.Count().unwrap_or(0) {
                let Ok(t) = tasks.get_Item(&VARIANT::from(i)) else { continue };
                if let Some((path, enabled, exe, args)) = read_task(&t) {
                    items.push(make_item(path, enabled, &exe, &args, sig, touch_free_names));
                }
            }
        }
    }
    items.sort_by_key(|i| i.name.to_lowercase());
    items
}

/// COM の失敗を OpError に(権限が足りないなら Win32(5) になり、失敗の帯に「管理者で再起動」が出る)
fn op_error(e: &windows::core::Error) -> OpError {
    OpError::from_code(win32_code(e))
}

/// タスクを有効・無効にする。name は一覧の StartupItem.name(タスクのパス)。Enabled を変えるだけ
pub fn set(name: &str, enabled: bool) -> Result<(), OpError> {
    check_name(name)?;
    let _com = Com::init();
    let svc = connect().map_err(|e| op_error(&e))?;
    // SAFETY: COM はこのスレッドで初期化した
    unsafe {
        let root = svc.GetFolder(&BSTR::from("\\")).map_err(|e| op_error(&e))?;
        let task = root.GetTask(&BSTR::from(name)).map_err(|e| op_error(&e))?;
        // 渡された文字列ではなく、実際に開いたタスクのパスで確かめ直す(`\\Microsoft\...` のような書き方で
        // `\Microsoft\` の下のタスクに届かないように)
        let actual = task.Path().map_err(|e| op_error(&e))?.to_string();
        if !actual.eq_ignore_ascii_case(name) {
            return Err(OpError::Blocked("このタスクは切り替えられません"));
        }
        check_name(&actual)?;
        task.SetEnabled(VARIANT_BOOL::from(enabled)).map_err(|e| op_error(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{E_ACCESSDENIED, VARIANT_FALSE, VARIANT_TRUE};

    #[test]
    fn folders_and_names() {
        assert!(skip_folder("\\Microsoft") && skip_folder("\\microsoft\\") && skip_folder("\\MICROSOFT"));
        assert!(!skip_folder("\\MicrosoftEdgeUpdate") && !skip_folder("\\") && !skip_folder("\\Vendor\\Microsoft"));
        assert!(check_name("\\Foo\\Bar").is_ok() && check_name("\\MicrosoftEdgeUpdateTaskMachineUA").is_ok());
        for bad in ["\\Microsoft\\Windows\\X", "\\microsoft\\Office\\Y", "Foo", "", "\\"] {
            assert!(matches!(check_name(bad), Err(OpError::Blocked(_))), "{bad}");
        }
    }

    #[test]
    fn commands() {
        let ex = |s: &str| s.replace("%ProgramFiles%", "C:\\Program Files").replace("%windir%", "C:\\Windows");
        assert_eq!(command_of("%ProgramFiles%\\A B\\a.exe", "/logon", &ex), "\"C:\\Program Files\\A B\\a.exe\" /logon");
        assert_eq!(command_of("\"C:\\x\\y.exe\"", "", &ex), "C:\\x\\y.exe");
        assert_eq!(command_of("%windir%\\system32\\cmd.exe", " /c %windir%\\a.cmd ", &ex), "C:\\Windows\\system32\\cmd.exe /c C:\\Windows\\a.cmd");
        assert_eq!(unquote("\"C:\\a b\\c.exe\""), "C:\\a b\\c.exe");
        assert_eq!(unquote("\"C:\\half"), "C:\\half");
        assert!(locate("cmd.exe").to_lowercase().ends_with("\\system32\\cmd.exe"));
        assert!(locate("powershell").to_lowercase().ends_with(".exe") || locate("powershell") == "powershell");
    }

    #[test]
    fn set_values_without_calling() {
        // 呼ばずに確かめる: 切り替えの値と、権限の失敗の形
        assert_eq!(VARIANT_BOOL::from(true), VARIANT_TRUE);
        assert_eq!(VARIANT_BOOL::from(false), VARIANT_FALSE);
        let denied = op_error(&windows::core::Error::from(E_ACCESSDENIED));
        assert!(denied.is_access_denied(), "{denied:?}");
        assert!(denied.message(false).contains("管理者で再起動"));
        // 触れない名前は署名を見ない(SignatureCache に問い合わせずに None)
        let sig = SignatureCache::new();
        assert_eq!(publisher("C:\\Games\\AceGame.exe", true, &sig, &["acegame.exe".into()]), None);
        assert_eq!(publisher("C:\\nothing\\x.exe", false, &sig, &[]), None);
    }

    #[test]
    fn list_reads() {
        // 読むだけ(タスクを変えない)。名前は出さず件数だけ
        let sig = SignatureCache::new();
        let items = list(&sig, &["csrss.exe".into()]);
        let on = items.iter().filter(|i| i.approved == Approved::Enabled).count();
        println!(
            "タスク スケジューラ: 全 {} 件 (有効 {on}、無効 {})、ファイルなし {}、発行元あり {}",
            items.len(),
            items.len() - on,
            items.iter().filter(|i| !i.exe_exists).count(),
            items.iter().filter(|i| i.publisher.is_some()).count(),
        );
        for i in &items {
            assert_eq!(i.location, StartupLocation::ScheduledTask);
            assert!(i.name.starts_with('\\'));
            assert!(!i.name.to_lowercase().starts_with("\\microsoft\\"), "\\Microsoft\\ の下が入った");
            assert!(i.readonly_reason.is_none() && i.exe_path.is_some());
        }
    }
}
