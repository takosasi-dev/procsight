//! サービス(担当 B。FR-41〜FR-44・D-34)。SCM の API で、開始・停止・再起動・開始の種類の変更だけを行う(INV-13)。
//! 種類は SERVICE_WIN32 だけ(ドライバは出さない)。状態の変化は 250 ミリ秒ごとに見て 30 秒で打ち切る。

use crate::jobs::{OpError, ServiceAction, ServiceRow, ServiceState, StartType};
use crate::util::{pcwstr, pwstr_to_string, wide, win32_code};
use std::time::Duration;
use windows::Win32::System::Services::{
    CloseServiceHandle, ENUM_SERVICE_STATUS_PROCESSW, ENUM_SERVICE_STATUSW, EnumDependentServicesW,
    EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfig2W,
    QueryServiceConfigW, QueryServiceStatusEx, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_ENUMERATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_ACCEPT_STOP, SERVICE_ACTIVE,
    SERVICE_AUTO_START, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_DELAYED_AUTO_START_INFO, SERVICE_DEMAND_START, SERVICE_DESCRIPTIONW, SERVICE_DISABLED,
    SERVICE_ENUMERATE_DEPENDENTS, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START_TYPE,
    SERVICE_STATE_ALL, SERVICE_STATUS_PROCESS, SERVICE_WIN32,
};
use windows::core::PCWSTR;

const ERROR_MORE_DATA: u32 = 234;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
const ERROR_SERVICE_DISABLED: u32 = 1058;
const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;
const POLL: Duration = Duration::from_millis(250);
const LIMIT_MS: u64 = 30_000;

struct Sc(SC_HANDLE);

impl Drop for Sc {
    fn drop(&mut self) {
        // SAFETY: OpenSCManagerW / OpenServiceW が返した有効なハンドルで、1 回だけ閉じる
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn manager() -> Result<Sc, OpError> {
    // SAFETY: 自分の PC の既定のデータベースを、一覧と接続の権限だけで開く
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE) }
        .map(Sc)
        .map_err(|e| OpError::from_code(win32_code(&e)))
}

fn open(m: &Sc, name: &str, access: u32) -> Result<Sc, OpError> {
    let n = wide(name);
    // SAFETY: n は 0 で終わる。m は開いたマネージャー
    unsafe { OpenServiceW(m.0, pcwstr(&n), access) }.map(Sc).map_err(|e| OpError::from_code(win32_code(&e)))
}

pub(crate) fn state_of(v: u32) -> ServiceState {
    match v {
        2 => ServiceState::StartPending,
        3 => ServiceState::StopPending,
        4 => ServiceState::Running,
        5 => ServiceState::ContinuePending,
        6 => ServiceState::PausePending,
        7 => ServiceState::Paused,
        _ => ServiceState::Stopped,
    }
}

fn start_type_of(v: SERVICE_START_TYPE, delayed: bool) -> Option<StartType> {
    Some(match v.0 {
        0 => StartType::Boot,
        1 => StartType::System,
        2 if delayed => StartType::AutoDelayed,
        2 => StartType::Auto,
        3 => StartType::Manual,
        4 => StartType::Disabled,
        _ => return None,
    })
}

/// QueryServiceConfig2W の結果を 8 バイトに揃えたバッファで
fn config2(s: &Sc, level: windows::Win32::System::Services::SERVICE_CONFIG) -> Option<Vec<u64>> {
    let mut need = 0u32;
    // SAFETY: 大きさを聞くだけ
    let _ = unsafe { QueryServiceConfig2W(s.0, level, None, &mut need) };
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u64; (need as usize).div_ceil(8)];
    // SAFETY: buf は need バイト以上で、u8 の切れ端として渡す
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8) };
    // SAFETY: bytes の長さを渡している
    unsafe { QueryServiceConfig2W(s.0, level, Some(bytes), &mut need) }.ok()?;
    Some(buf)
}

/// (開始の種類、説明、アカウント)。読めなければ (None, 空, 空)
fn config_of(m: &Sc, name: &str) -> (Option<StartType>, String, String) {
    let Ok(s) = open(m, name, SERVICE_QUERY_CONFIG) else { return (None, String::new(), String::new()) };
    let mut need = 0u32;
    // SAFETY: 大きさを聞くだけ
    let _ = unsafe { QueryServiceConfigW(s.0, None, 0, &mut need) };
    let mut buf = vec![0u64; (need as usize).div_ceil(8).max(1)];
    // SAFETY: buf は need バイト以上。結果の文字列は buf の中を指し、buf が生きている間だけ読む
    let (start, account) = match unsafe { QueryServiceConfigW(s.0, Some(buf.as_mut_ptr().cast()), need, &mut need) } {
        Ok(()) => {
            // SAFETY: 成功したので先頭は QUERY_SERVICE_CONFIGW
            let c = unsafe { &*(buf.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
            (Some(c.dwStartType), pwstr_to_string(c.lpServiceStartName))
        }
        Err(_) => (None, String::new()),
    };
    let delayed = config2(&s, SERVICE_CONFIG_DELAYED_AUTO_START_INFO)
        // SAFETY: 結果の先頭は SERVICE_DELAYED_AUTO_START_INFO
        .map(|b| unsafe { (*(b.as_ptr() as *const SERVICE_DELAYED_AUTO_START_INFO)).fDelayedAutostart.as_bool() })
        .unwrap_or(false);
    let desc = config2(&s, SERVICE_CONFIG_DESCRIPTION)
        // SAFETY: 結果の先頭は SERVICE_DESCRIPTIONW で、文字列は同じバッファの中
        .map(|b| pwstr_to_string(unsafe { (*(b.as_ptr() as *const SERVICE_DESCRIPTIONW)).lpDescription }))
        .unwrap_or_default();
    (start.and_then(|t| start_type_of(t, delayed)), desc, account)
}

pub fn load() -> Result<Vec<ServiceRow>, OpError> {
    let m = manager()?;
    let mut out = Vec::new();
    let mut resume = 0u32;
    let mut buf = vec![0u64; 64 * 1024 / 8];
    loop {
        let (mut need, mut n) = (0u32, 0u32);
        // SAFETY: buf を u8 の切れ端として渡す
        let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8) };
        // SAFETY: bytes の長さを渡し、書かれた n 件だけを読む。resume は続きの位置
        let r = unsafe {
            EnumServicesStatusExW(
                m.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(bytes),
                &mut need,
                &mut n,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        let more = match r {
            Ok(()) => false,
            Err(e) if win32_code(&e) == ERROR_MORE_DATA => true,
            Err(e) => return Err(OpError::from_code(win32_code(&e))),
        };
        // SAFETY: 先頭から n 件の ENUM_SERVICE_STATUS_PROCESSW が並び、文字列は buf の中を指す
        let entries = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, n as usize) };
        for e in entries {
            let st: &SERVICE_STATUS_PROCESS = &e.ServiceStatusProcess;
            let name = pwstr_to_string(e.lpServiceName);
            let (start_type, description, account) = config_of(&m, &name);
            out.push(ServiceRow {
                display_name: pwstr_to_string(e.lpDisplayName),
                state: state_of(st.dwCurrentState.0),
                start_type,
                pid: st.dwProcessId,
                description,
                account,
                accepts_stop: st.dwControlsAccepted & SERVICE_ACCEPT_STOP != 0,
                name,
            });
        }
        if !more {
            return Ok(out);
        }
        if n == 0 {
            buf = vec![0u64; (need as usize).div_ceil(8) + 1];
        }
    }
}

fn dependents_of(s: &Sc) -> Result<Vec<(String, String)>, OpError> {
    let (mut need, mut n) = (0u32, 0u32);
    // SAFETY: 大きさを聞くだけ。依存が無ければ成功して n = 0
    match unsafe { EnumDependentServicesW(s.0, SERVICE_ACTIVE, None, 0, &mut need, &mut n) } {
        Ok(()) => return Ok(Vec::new()),
        Err(e) if win32_code(&e) == ERROR_MORE_DATA || win32_code(&e) == ERROR_INSUFFICIENT_BUFFER => {}
        Err(e) => return Err(OpError::from_code(win32_code(&e))),
    }
    let mut buf = vec![0u64; (need as usize).div_ceil(8)];
    // SAFETY: buf は need バイト以上。書かれた n 件だけを読む
    unsafe { EnumDependentServicesW(s.0, SERVICE_ACTIVE, Some(buf.as_mut_ptr().cast()), need, &mut need, &mut n) }
        .map_err(|e| OpError::from_code(win32_code(&e)))?;
    // SAFETY: 先頭から n 件の ENUM_SERVICE_STATUSW が並び、文字列は buf の中を指す
    let entries = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUSW, n as usize) };
    Ok(entries.iter().map(|e| (pwstr_to_string(e.lpServiceName), pwstr_to_string(e.lpDisplayName))).collect())
}

/// 動いている依存のサービス (名前, 表示名)。並びは止めてよい順(EnumDependentServicesW は開始の逆順で返す)
pub fn dependents(name: &str) -> Result<Vec<(String, String)>, OpError> {
    let m = manager()?;
    let s = open(&m, name, SERVICE_ENUMERATE_DEPENDENTS)?;
    dependents_of(&s)
}

fn current_state(s: &Sc) -> Result<ServiceState, OpError> {
    let mut st = SERVICE_STATUS_PROCESS::default();
    let mut need = 0u32;
    // SAFETY: st はこの関数の中の構造体で、その大きさの切れ端として渡す
    let bytes = unsafe {
        std::slice::from_raw_parts_mut((&mut st as *mut SERVICE_STATUS_PROCESS).cast::<u8>(), size_of::<SERVICE_STATUS_PROCESS>())
    };
    // SAFETY: bytes の長さを渡している
    unsafe { QueryServiceStatusEx(s.0, SC_STATUS_PROCESS_INFO, Some(bytes), &mut need) }
        .map_err(|e| OpError::from_code(win32_code(&e)))?;
    Ok(state_of(st.dwCurrentState.0))
}

/// 250 ミリ秒ごとに見て、want になるのを 30 秒まで待つ(E-31)
fn wait_for(s: &Sc, want: ServiceState) -> Result<(), OpError> {
    let start = crate::util::tick_ms();
    loop {
        if current_state(s)? == want {
            return Ok(());
        }
        if crate::util::tick_ms() - start >= LIMIT_MS {
            return Err(OpError::ServiceTimeout);
        }
        std::thread::sleep(POLL);
    }
}

fn stop_one(m: &Sc, name: &str) -> Result<(), OpError> {
    use windows::Win32::System::Services::{SERVICE_CONTROL_STOP, SERVICE_STATUS, SERVICE_STOP, ControlService};
    let s = open(m, name, SERVICE_STOP | SERVICE_QUERY_STATUS)?;
    let mut st = SERVICE_STATUS::default();
    // SAFETY: st はこの関数の中の構造体
    match unsafe { ControlService(s.0, SERVICE_CONTROL_STOP, &mut st) } {
        Ok(()) => {}
        Err(e) if win32_code(&e) == ERROR_SERVICE_NOT_ACTIVE => return Ok(()),
        Err(e) => return Err(OpError::from_code(win32_code(&e))),
    }
    wait_for(&s, ServiceState::Stopped)
}

fn start_one(m: &Sc, name: &str) -> Result<(), OpError> {
    use windows::Win32::System::Services::{SERVICE_START, StartServiceW};
    let s = open(m, name, SERVICE_START | SERVICE_QUERY_STATUS)?;
    // SAFETY: 引数なしで開始する
    match unsafe { StartServiceW(s.0, None) } {
        Ok(()) => {}
        Err(e) if win32_code(&e) == ERROR_SERVICE_ALREADY_RUNNING => return Ok(()),
        Err(e) if win32_code(&e) == ERROR_SERVICE_DISABLED => {
            return Err(OpError::Blocked("開始の種類が無効のため開始できません")); // E-32
        }
        Err(e) => return Err(OpError::from_code(win32_code(&e))),
    }
    wait_for(&s, ServiceState::Running)
}

/// 停止は依存から先に止める。状態は 250 ミリ秒ごとに見て 30 秒で打ち切る。
/// `confirmed` は確認の窓に出した依存の名前。今動いている依存がその外にあれば何も止めない(INV-5)
pub fn control(name: &str, action: ServiceAction, confirmed: &[String]) -> Result<(), OpError> {
    let m = manager()?;
    if matches!(action, ServiceAction::Stop | ServiceAction::Restart) {
        let deps = dependents_of(&open(&m, name, SERVICE_ENUMERATE_DEPENDENTS)?)?;
        if deps.iter().any(|(dep, _)| !confirmed.iter().any(|c| c.eq_ignore_ascii_case(dep))) {
            return Err(OpError::Blocked("確認の後に、動いている依存のサービスが増えました。もう一度確かめてください"));
        }
        for (dep, _) in &deps {
            stop_one(&m, dep)?;
        }
        stop_one(&m, name)?;
    }
    if matches!(action, ServiceAction::Start | ServiceAction::Restart) {
        start_one(&m, name)?;
    }
    Ok(())
}

/// ChangeServiceConfigW に渡す開始の種類と、遅延開始の印(None は変えない)
fn start_type_args(t: StartType) -> Option<(SERVICE_START_TYPE, Option<bool>)> {
    Some(match t {
        StartType::Auto => (SERVICE_AUTO_START, Some(false)),
        StartType::AutoDelayed => (SERVICE_AUTO_START, Some(true)),
        StartType::Manual => (SERVICE_DEMAND_START, None),
        StartType::Disabled => (SERVICE_DISABLED, None),
        StartType::Boot | StartType::System => return None,
    })
}

pub fn set_start_type(name: &str, start_type: StartType) -> Result<(), OpError> {
    use windows::Win32::System::Services::{
        ChangeServiceConfig2W, ChangeServiceConfigW, ENUM_SERVICE_TYPE, SERVICE_CHANGE_CONFIG, SERVICE_ERROR,
        SERVICE_NO_CHANGE,
    };
    let (ty, delayed) = start_type_args(start_type).ok_or(OpError::Blocked("この開始の種類には変えられません"))?;
    let m = manager()?;
    let s = open(&m, name, SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG)?;
    // SAFETY: 開始の種類だけを変え、他は SERVICE_NO_CHANGE と null で変えない
    unsafe {
        ChangeServiceConfigW(
            s.0,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            ty,
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
    .map_err(|e| OpError::from_code(win32_code(&e)))?;
    if let Some(d) = delayed {
        let info = SERVICE_DELAYED_AUTO_START_INFO { fDelayedAutostart: d.into() };
        // SAFETY: info はこの関数の中の構造体
        unsafe { ChangeServiceConfig2W(s.0, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, Some((&info as *const SERVICE_DELAYED_AUTO_START_INFO).cast())) }
            .map_err(|e| OpError::from_code(win32_code(&e)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_dependents_read() {
        let rows = load().unwrap();
        let running = rows.iter().filter(|r| r.state == ServiceState::Running).count();
        let delayed = rows.iter().filter(|r| r.start_type == Some(StartType::AutoDelayed)).count();
        let unknown = rows.iter().filter(|r| r.start_type.is_none()).count();
        println!("サービス: 全 {} 件、実行中 {running}、遅延開始 {delayed}、開始の種類を読めない {unknown}", rows.len());
        assert!(rows.len() > 50);
        assert!(rows.iter().all(|r| !r.name.is_empty()));
        // 読むだけ。標準の権限では依存の一覧を断るサービスがある(RpcSs は 5)ので、読める物が 1 つあればよい
        let (mut read, mut with_deps) = (0, 0);
        for r in rows.iter().filter(|r| r.state == ServiceState::Running) {
            if let Ok(d) = dependents(&r.name) {
                read += 1;
                with_deps += usize::from(!d.is_empty());
            }
        }
        println!("依存の一覧: 読めた {read} 件、動いている依存がある {with_deps} 件");
        assert!(read > 0 && with_deps > 0);
    }

    #[test]
    fn values_without_writing() {
        assert_eq!(start_type_args(StartType::AutoDelayed), Some((SERVICE_AUTO_START, Some(true))));
        assert_eq!(start_type_args(StartType::Auto), Some((SERVICE_AUTO_START, Some(false))));
        assert_eq!(start_type_args(StartType::Disabled), Some((SERVICE_DISABLED, None)));
        assert_eq!(start_type_args(StartType::Boot), None);
        assert_eq!(start_type_of(SERVICE_AUTO_START, true), Some(StartType::AutoDelayed));
        assert_eq!(start_type_of(SERVICE_DEMAND_START, true), Some(StartType::Manual));
        assert_eq!(state_of(4), ServiceState::Running);
        assert_eq!(state_of(1), ServiceState::Stopped);
    }
}
