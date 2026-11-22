//! ProcessGate(仕様書 §9.4・D-5・INV-1)。他のプロセスを開く呼び出し(OpenProcess・モジュールの
//! スナップショット・DuplicateHandle)はこのファイルの中だけに書く。触れないプロセスはここで必ず断り、
//! 開いた後に開始時刻を確かめ(D-8)、開いた回数を PID ごとに数える(AC-30)。
#![allow(clippy::disallowed_methods)] // このファイルだけが関門。clippy.toml で外では禁じている

use crate::util::win32_code;
use procsight_core::ProcId;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock};
use windows::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError {
    TouchFree,
    Replaced,
    Gone,
    AccessDenied,
    Protected,
    Other(u32),
}

impl GateError {
    pub(crate) fn from_code(code: u32, elevated: bool) -> GateError {
        match code {
            5 if elevated => GateError::Protected,
            5 => GateError::AccessDenied,
            87 => GateError::Gone,
            c => GateError::Other(c),
        }
    }
}

/// 関門で開いたプロセスのハンドル。Drop で閉じる
pub struct OwnedProcess {
    handle: HANDLE,
    pub id: ProcId,
}

impl OwnedProcess {
    pub fn raw(&self) -> HANDLE {
        self.handle
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        // SAFETY: handle は OpenProcess が返した有効なハンドルで、この型だけが持ち、ここで 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// 複製したハンドル・スナップショットのハンドル。Drop で閉じる
pub struct OwnedHandle(HANDLE);

impl OwnedHandle {
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: 中身は DuplicateHandle / CreateToolhelp32Snapshot が返した有効なハンドルで、ここで 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct ProcessGate {
    touch_free: RwLock<HashSet<ProcId>>,
    touch_stamp: std::sync::atomic::AtomicU64,
    counts: Mutex<HashMap<u32, u32>>,
    refused_touch_free: Mutex<u64>,
    self_pid: u32,
    elevated: bool,
}

impl ProcessGate {
    pub fn new(self_pid: u32, elevated: bool) -> ProcessGate {
        ProcessGate {
            touch_free: RwLock::new(HashSet::new()),
            touch_stamp: std::sync::atomic::AtomicU64::new(0),
            counts: Mutex::new(HashMap::new()),
            refused_touch_free: Mutex::new(0),
            self_pid,
            elevated,
        }
    }

    pub fn self_pid(&self) -> u32 {
        self.self_pid
    }

    pub fn elevated(&self) -> bool {
        self.elevated
    }

    /// Collector が毎回の取得で作った触れない集合に差し替える(FR-22)
    pub fn set_touch_free(&self, set: HashSet<ProcId>) {
        self.set_touch_free_at(set, crate::util::tick_ms());
    }

    /// `taken_at_ms` は集合の元にした SPI を読み始めた時刻(単調な時計)。今の集合より古い物では上書きしない
    /// (Collector と Worker の作り直しが前後しても、新しい方が残る)
    pub fn set_touch_free_at(&self, set: HashSet<ProcId>, taken_at_ms: u64) {
        let mut cur = self.touch_free.write().unwrap_or_else(|e| e.into_inner());
        let stamp = self.touch_stamp.load(std::sync::atomic::Ordering::SeqCst);
        if taken_at_ms < stamp {
            return;
        }
        self.touch_stamp.store(taken_at_ms, std::sync::atomic::Ordering::SeqCst);
        *cur = set;
    }

    pub fn is_touch_free(&self, id: ProcId) -> bool {
        let set = self.touch_free.read().unwrap_or_else(|e| e.into_inner());
        // 同じ PID の物があれば、開始時刻が違っても断る(安全側)
        set.contains(&id) || set.iter().any(|t| t.pid == id.pid)
    }

    fn check(&self, id: ProcId) -> Result<(), GateError> {
        if self.is_touch_free(id) {
            *self.refused_touch_free.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            return Err(GateError::TouchFree);
        }
        Ok(())
    }

    fn count(&self, pid: u32) {
        *self.counts.lock().unwrap_or_else(|e| e.into_inner()).entry(pid).or_insert(0) += 1;
    }

    /// 触れない集合に入っていれば開かずに Err(TouchFree)。access には必ず PROCESS_QUERY_LIMITED_INFORMATION を足して開き、
    /// GetProcessTimes の開始時刻が id.create_time と違えば閉じて Err(Replaced)。開けたら回数を数える
    pub fn open(&self, id: ProcId, access: PROCESS_ACCESS_RIGHTS) -> Result<OwnedProcess, GateError> {
        self.check(id)?;
        // SAFETY: OpenProcess は引数の値だけを読む。返ったハンドルはすぐ OwnedProcess に入れ、Drop で閉じる
        let handle = unsafe { OpenProcess(access | PROCESS_QUERY_LIMITED_INFORMATION, false, id.pid) }
            .map_err(|e| GateError::from_code(win32_code(&e), self.elevated))?;
        let owned = OwnedProcess { handle, id };
        self.count(id.pid);
        let mut create = FILETIME::default();
        let (mut exit, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: handle は今開いた有効なハンドル。4 つの FILETIME はこの関数の中の変数を指す
        unsafe { GetProcessTimes(owned.raw(), &mut create, &mut exit, &mut kernel, &mut user) }
            .map_err(|e| GateError::from_code(win32_code(&e), self.elevated))?;
        let ct = ((create.dwHighDateTime as u64) << 32) | create.dwLowDateTime as u64;
        if ct != id.create_time {
            return Err(GateError::Replaced); // owned はここで閉じる
        }
        Ok(owned)
    }

    /// PID を渡すだけで中でプロセスを開く API の前に呼ぶ。触れない集合の確かめと、回数の数え上げだけを行う
    pub fn permit(&self, id: ProcId, _what: &'static str) -> Result<(), GateError> {
        self.check(id)?;
        self.count(id.pid);
        Ok(())
    }

    /// DuplicateHandle はここでだけ呼ぶ。source は open で開いた物に限る(PROCESS_DUP_HANDLE を付けて開くこと)
    pub fn duplicate_from(&self, source: &OwnedProcess, handle_value: usize) -> Result<OwnedHandle, GateError> {
        self.check(source.id)?;
        let mut out = HANDLE::default();
        // SAFETY: source は関門で開いた有効なハンドル。handle_value は相手のプロセスの中の値として渡すだけで、
        // 自分の側では DuplicateHandle が作った out だけを使う
        unsafe {
            DuplicateHandle(
                source.raw(),
                HANDLE(handle_value as *mut core::ffi::c_void),
                GetCurrentProcess(),
                &mut out,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }
        .map_err(|e| GateError::from_code(win32_code(&e), self.elevated))?;
        Ok(OwnedHandle(out))
    }

    /// モジュールのスナップショット(D-21)。先に open で開いて開始時刻を確かめ、そのハンドルを持ったまま
    /// CreateToolhelp32Snapshot を呼ぶ(持っている間は PID が使い回されない。INV-6)
    pub fn module_snapshot(&self, id: ProcId) -> Result<OwnedHandle, GateError> {
        let _hold = self.open(id, PROCESS_QUERY_LIMITED_INFORMATION)?;
        self.permit(id, "module_snapshot")?;
        // SAFETY: 引数の値だけを渡す。返ったハンドルはすぐ OwnedHandle に入れ、Drop で閉じる
        let h = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, id.pid) }
            .map_err(|e| GateError::from_code(win32_code(&e), self.elevated))?;
        Ok(OwnedHandle(h))
    }

    pub fn open_counts(&self) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> =
            self.counts.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(k, v)| (*k, *v)).collect();
        v.sort();
        v
    }

    /// 関門が触れないプロセスへの依頼を断った回数(開いた回数ではない)
    pub fn refused_touch_free(&self) -> u64 {
        *self.refused_touch_free.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 終了時のログの要約(§9.8)。`touch_free_pids` は今までに触れないとされた PID の全部
    pub fn summary(&self, touch_free_pids: &HashSet<u32>) -> (u64, u64) {
        let counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let total = counts.values().map(|v| *v as u64).sum();
        let tf = counts.iter().filter(|(p, _)| touch_free_pids.contains(p)).map(|(_, v)| *v as u64).sum();
        (tf, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_free_is_refused_without_opening() {
        let g = ProcessGate::new(1, false);
        let id = ProcId { pid: 4242, create_time: 7 };
        g.set_touch_free(HashSet::from([id]));
        assert_eq!(g.open(id, PROCESS_QUERY_LIMITED_INFORMATION).err(), Some(GateError::TouchFree));
        assert_eq!(g.permit(ProcId { pid: 4242, create_time: 8 }, "x"), Err(GateError::TouchFree));
        assert!(g.open_counts().is_empty());
        assert_eq!(g.refused_touch_free(), 2);
    }

    #[test]
    fn replaced_when_start_time_differs() {
        let me = std::process::id();
        let g = ProcessGate::new(me, false);
        let r = g.open(ProcId { pid: me, create_time: 1 }, PROCESS_QUERY_LIMITED_INFORMATION);
        assert_eq!(r.err(), Some(GateError::Replaced));
        assert_eq!(g.open_counts(), vec![(me, 1)]);
    }
}
