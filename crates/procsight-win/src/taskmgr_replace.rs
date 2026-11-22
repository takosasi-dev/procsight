//! Ctrl+Shift+Esc の置き換え(担当 B。FR-53〜FR-56・D-38〜D-43)。Image File Execution Options の taskmgr.exe の
//! Debugger の値を読み書きするのはこのファイルだけ(INV-13・INV-15)。WOW6432Node の側は読むだけ。
//! 書く・消す処理はテストで動かさない(順番の決まりは差し替えた関数で確かめる)。

use crate::gate::ProcessGate;
use crate::jobs::{OpError, ReplaceInfo, Target};
use crate::startup::{
    ERROR_FILE_NOT_FOUND, create_key, delete_value, open_key, read_value, reg_string, reg_string_bytes, write_value,
};
use procsight_core::ProcId;
use procsight_core::taskmgr::{ReplaceBlock, ReplaceStatus, check_can_enable, debugger_value, status_of};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SAM_FLAGS,
    REG_SZ,
};

const IFEO_TASKMGR: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\taskmgr.exe";
const DEBUGGER: &str = "Debugger";
const TASKMGR: &str = "Taskmgr.exe";
const WAIT_NEW_MS: u64 = 5000;

/// view は KEY_WOW64_64KEY(普段のビュー)か KEY_WOW64_32KEY(WOW6432Node)
fn read_debugger(view: REG_SAM_FLAGS) -> Option<String> {
    let k = open_key(HKEY_LOCAL_MACHINE, IFEO_TASKMGR, KEY_QUERY_VALUE | view).ok()?;
    match read_value(&k, DEBUGGER) {
        Ok(Some((ty, data))) if ty == REG_SZ || ty == REG_EXPAND_SZ => Some(reg_string(&data)).filter(|s| !s.trim().is_empty()),
        _ => None,
    }
}

/// 普段のビューだけに書く(キーが無ければ作る)
fn write_debugger(value: &str) -> Result<(), OpError> {
    let k = create_key(HKEY_LOCAL_MACHINE, IFEO_TASKMGR, KEY_SET_VALUE | KEY_WOW64_64KEY).map_err(OpError::from_code)?;
    write_value(&k, DEBUGGER, REG_SZ, &reg_string_bytes(value)).map_err(OpError::from_code)
}

/// 値だけを消す。キーと他の値には触らない
fn delete_debugger() -> Result<(), OpError> {
    match open_key(HKEY_LOCAL_MACHINE, IFEO_TASKMGR, KEY_SET_VALUE | KEY_WOW64_64KEY) {
        Ok(k) => delete_value(&k, DEBUGGER).map_err(OpError::from_code),
        Err(ERROR_FILE_NOT_FOUND) => Ok(()),
        Err(c) => Err(OpError::from_code(c)),
    }
}

fn block_now(final_path: &str) -> Result<(), ReplaceBlock> {
    check_can_enable(crate::elevation::is_elevated(), final_path, &crate::util::program_files())
}

pub fn read_info() -> ReplaceInfo {
    let self_final_path = crate::elevation::self_final_path();
    ReplaceInfo {
        debugger: read_debugger(KEY_WOW64_64KEY),
        debugger_wow64: read_debugger(KEY_WOW64_32KEY),
        block: block_now(&self_final_path).err(),
        self_final_path,
    }
}

/// 昇格と D-39 を確かめてから書く。戻り値は書く前の別の道具の値(無ければ空。D-41)
pub fn enable() -> Result<String, OpError> {
    let final_path = crate::elevation::self_final_path();
    // INV-15: 書く直前に確かめる
    block_now(&final_path).map_err(|b| OpError::Blocked(b.message()))?;
    let previous = previous_to_remember(read_debugger(KEY_WOW64_64KEY).as_deref(), &final_path);
    write_debugger(&debugger_value(&final_path))?;
    Ok(previous)
}

/// 覚えておく前の値: 別の道具の値だけ(ProcSight の古い登録は覚えない)
fn previous_to_remember(current: Option<&str>, final_path: &str) -> String {
    match status_of(current, final_path) {
        ReplaceStatus::Other(v) => v,
        _ => String::new(),
    }
}

/// disable で何をするか
#[derive(Debug, PartialEq)]
enum DisableAction {
    Restore(String),
    Delete,
    /// 今の値が ProcSight の物でない(別の道具が書き直した)ので触らない
    Leave,
}

fn disable_action(current: Option<&str>, final_path: &str, previous: &str) -> DisableAction {
    if matches!(status_of(current, final_path), ReplaceStatus::Other(_)) {
        return DisableAction::Leave;
    }
    if previous.trim().is_empty() { DisableAction::Delete } else { DisableAction::Restore(previous.to_string()) }
}

/// `previous` があればそれに戻し、無ければ値を消す。キーと他の値には触らない
pub fn disable(previous: &str) -> Result<(), OpError> {
    if !crate::elevation::is_elevated() {
        return Err(OpError::Blocked(ReplaceBlock::NotElevated.message()));
    }
    let final_path = crate::elevation::self_final_path();
    match disable_action(read_debugger(KEY_WOW64_64KEY).as_deref(), &final_path, previous) {
        DisableAction::Restore(v) => write_debugger(&v),
        DisableAction::Delete => delete_debugger(),
        DisableAction::Leave => Ok(()),
    }
}

/// D-40 の (2)〜(5)。(5) の戻しは (2)〜(4) が失敗しても必ず行う。戻り値は (起動の結果, 戻せなかった)
fn swap_and_launch(
    remove: impl FnOnce() -> Result<(), OpError>,
    launch: impl FnOnce() -> Result<(), OpError>,
    wait: impl FnOnce(),
    restore: impl FnOnce() -> bool,
) -> (Result<(), OpError>, bool) {
    let result = remove().and_then(|()| {
        launch()?;
        wait();
        Ok(())
    });
    let restored = restore();
    (result, !restored)
}

/// 起動の前になかった Taskmgr.exe が SPI に現れるのを最大 5 秒待つ
fn wait_new_taskmgr(before: &HashSet<ProcId>) {
    let start = crate::util::tick_ms();
    while crate::util::tick_ms() - start < WAIT_NEW_MS {
        if crate::nt::spi::pids_named(TASKMGR).iter().any(|id| !before.contains(id)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// D-40 で値を外している間は真。ProcSight を閉じるときに、戻し終わるのを待つ(Backend::shutdown)
static SWAPPING: AtomicBool = AtomicBool::new(false);

pub fn swap_in_progress() -> bool {
    SWAPPING.load(Ordering::SeqCst)
}

/// FR-23・FR-55。置き換えがオフなら shell::launch_taskmgr。オンなら D-40 の手順(戻すのは失敗しても必ず行う)。
/// 本物がすでに動いていれば前面に出す。戻り値の Some は「値を戻せなかった」(E-45)ときの戻す値
pub fn open_task_manager(gate: &ProcessGate) -> (Result<(), OpError>, Option<String>) {
    open_task_manager_inner(gate, false)
}

/// `any_value` が偽なら、値が ProcSight を指しているときだけ D-40 を行う(別の道具の値は外さない。INV-13・INV-15)。
/// 真は probe の `ifeo` の (4) だけ(Phase 0 の試し)
fn open_task_manager_inner(gate: &ProcessGate, any_value: bool) -> (Result<(), OpError>, Option<String>) {
    let running = crate::nt::spi::pids_named(TASKMGR);
    if let Some(id) = running.iter().find(|id| crate::window_scan::first_visible_window(id.pid).is_some()) {
        // レジストリに触らず、その窓を前面に出す(FR-17)
        return (crate::ops::bring_to_front(gate, &Target { id: *id, name: TASKMGR.into() }), None);
    }
    // (1) 今の値を読んで覚える
    let final_path = crate::elevation::self_final_path();
    let ours = |v: &String| matches!(status_of(Some(v), &final_path), ReplaceStatus::OnSelf { .. });
    let Some(saved) = read_debugger(KEY_WOW64_64KEY).filter(|v| any_value || ours(v)) else {
        return (crate::shell::launch_taskmgr(), None);
    };
    if !crate::elevation::is_elevated() {
        return (Err(OpError::Blocked("管理者で再起動すると開けます")), None);
    }
    let before: HashSet<ProcId> = running.into_iter().collect();
    SWAPPING.store(true, Ordering::SeqCst);
    let (result, failed) = swap_and_launch(
        delete_debugger,
        crate::shell::launch_taskmgr,
        || wait_new_taskmgr(&before),
        || write_debugger(&saved).is_ok() && read_debugger(KEY_WOW64_64KEY).as_deref() == Some(saved.as_str()),
    );
    SWAPPING.store(false, Ordering::SeqCst);
    (result, failed.then_some(saved))
}

/// E-45 の「もう一度」。D-40 で戻せなかった値を書き戻す。書くのは ProcSight を指す値だけ(INV-15)
pub fn restore(value: &str) -> Result<(), OpError> {
    let final_path = crate::elevation::self_final_path();
    if !matches!(status_of(Some(value), &final_path), ReplaceStatus::OnSelf { .. }) {
        return Err(OpError::Blocked("戻す値が ProcSight を指していません"));
    }
    if !crate::elevation::is_elevated() {
        return Err(OpError::Blocked(ReplaceBlock::NotElevated.message()));
    }
    write_debugger(value)
}

/// probe の `ifeo` だけが使う(Phase 0 の試し。INV-15 の例外として仕様書 §9.5 が認める)。値をそのまま書くか、None なら消す
pub fn probe_set_raw(value: Option<&str>) -> Result<(), OpError> {
    match value {
        Some(v) => write_debugger(v),
        None => delete_debugger(),
    }
}

/// probe の `ifeo` の (4)。値が ProcSight の物でなくても D-40 の手順を試す
pub fn probe_open_task_manager(gate: &ProcessGate) -> (Result<(), OpError>, Option<String>) {
    open_task_manager_inner(gate, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn read_info_reads() {
        let info = read_info();
        assert!(!info.self_final_path.is_empty());
        // テストの exe は Program Files の下に無いので、必ずどちらかの理由で断られる
        assert!(info.block.is_some());
        println!(
            "置き換え: 普段のビューの値 {}、WOW6432Node の値 {}",
            if info.debugger.is_some() { "あり" } else { "なし" },
            if info.debugger_wow64.is_some() { "あり" } else { "なし" }
        );
    }

    #[test]
    fn restore_always_runs() {
        let log = RefCell::new(Vec::new());
        let step = |s: &'static str, ok: bool| {
            let log = &log;
            move || {
                log.borrow_mut().push(s);
                if ok { Ok(()) } else { Err(OpError::Win32(5)) }
            }
        };
        // 起動に失敗しても戻す
        let (r, failed) = swap_and_launch(step("remove", true), step("launch", false), || log.borrow_mut().push("wait"), || {
            log.borrow_mut().push("restore");
            true
        });
        assert!(r.is_err() && !failed);
        assert_eq!(*log.borrow(), ["remove", "launch", "restore"]);
        log.borrow_mut().clear();
        // 消すのに失敗したら起動せず、戻しは行う。戻せなければ true
        let (r, failed) = swap_and_launch(step("remove", false), step("launch", true), || log.borrow_mut().push("wait"), || {
            log.borrow_mut().push("restore");
            false
        });
        assert!(r.is_err() && failed);
        assert_eq!(*log.borrow(), ["remove", "restore"]);
    }

    #[test]
    fn remember_and_disable_rules() {
        let me = "C:\\Program Files\\ProcSight\\ProcSight.exe";
        let pe = "\"C:\\Tools\\procexp64.exe\"";
        assert_eq!(previous_to_remember(Some(pe), me), pe);
        assert_eq!(previous_to_remember(Some(&debugger_value(me)), me), "");
        assert_eq!(previous_to_remember(None, me), "");
        assert_eq!(disable_action(Some(&debugger_value(me)), me, pe), DisableAction::Restore(pe.into()));
        assert_eq!(disable_action(Some(&debugger_value(me)), me, ""), DisableAction::Delete);
        assert_eq!(disable_action(Some(pe), me, ""), DisableAction::Leave);
        assert_eq!(disable_action(None, me, ""), DisableAction::Delete);
    }
}
