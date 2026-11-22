//! ProcSight の Win32 と NT の API を呼ぶ部分(仕様書 §2)。unsafe はこの crate の中にだけ書く(INV-8)。
//! 画面(procsight-app)は `Backend` に Job を送り、JobResult と Snapshot を受けるだけ(INV-9)。
//! 担当の分け方と契約は docs/INTERFACES.md。

pub mod backend;
pub mod clipboard;
pub mod collector;
pub mod details;
pub mod elevation;
pub mod gate;
pub mod icons;
pub mod jobs;
pub mod mem;
pub mod net;
pub mod nt;
pub mod ops;
pub mod pdh;
pub mod services;
pub mod shell;
pub mod single_instance;
pub mod startup;
pub mod startup_store;
pub mod startup_tasks;
pub mod static_info;
pub mod taskmgr_replace;
pub mod util;
pub mod window_scan;
pub mod worker;

pub use backend::{ApiAvailability, Backend, BackendConfig, Diagnostics};
pub use gate::{GateError, ProcessGate};
pub use jobs::*;

/// 起動の最初に呼ぶ(INV-10)。後から読む DLL を System32 からだけにする
pub fn harden_dll_search() {
    use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
    // SAFETY: 引数の値だけを渡す。失敗しても(古い Windows)起動は続ける
    let _ = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
}

/// 画面が起動時に 1 回読む環境の値(読むだけ)
#[derive(Clone, Debug)]
pub struct EnvInfo {
    pub self_pid: u32,
    pub elevated: bool,
    pub system_root: String,
    pub program_files: String,
    /// ProcSight と同じユーザーの行を選ぶため(FR-5)。ログと設定には書かない(INV-7)
    pub user: Option<String>,
    pub logical_cpus: u32,
    pub processor_groups: u16,
    pub utc_offset_minutes: i32,
    /// ProcSight.exe の実体のパス(GetFinalPathNameByHandleW の後)
    pub self_final_path: String,
    /// パフォーマンスの CPU のページの見出し
    pub cpu_name: Option<String>,
    /// CPU の作り(コア・ソケット・キャッシュ)。取れなければ None(v0.2.0)
    pub cpu_info: Option<CpuInfo>,
}

/// GetLogicalProcessorInformationEx から読む CPU の作り。クロックは読まない(非スコープ。RigSight の担当)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuInfo {
    pub sockets: u32,
    pub cores: u32,
    pub logical: u32,
    /// キャッシュの合計(バイト)。0 は無い・取れない
    pub l1: u64,
    pub l2: u64,
    pub l3: u64,
}

pub fn env_info() -> EnvInfo {
    EnvInfo {
        self_pid: util::self_pid(),
        elevated: elevation::is_elevated(),
        system_root: util::system_root(),
        program_files: util::program_files(),
        user: elevation::current_user(),
        logical_cpus: util::logical_cpus(),
        processor_groups: util::processor_groups(),
        utc_offset_minutes: util::local_offset_minutes(),
        self_final_path: elevation::self_final_path(),
        cpu_name: util::cpu_name(),
        cpu_info: util::cpu_info(),
    }
}
