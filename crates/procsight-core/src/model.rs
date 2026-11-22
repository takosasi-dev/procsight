//! 取得した生の値(RawSnapshot)と、画面に出す行(ProcessRow / Snapshot)の型。
//! 型の形は仕様書 §9.2。§9.2 で「Phase 1 で決める」とされた型(Totals など)もここで決めた。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// プロセスの識別子。開始時刻は FILETIME(100 ナノ秒単位、1601 年から)。PID の使い回しを見分ける(D-8)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Default)]
pub struct ProcId {
    pub pid: u32,
    pub create_time: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawThread {
    pub tid: u32,
    pub start_address: u64,
    pub priority: i32,
    pub base_priority: i32,
    pub state: u32,
    pub wait_reason: u32,
    pub kernel_time: Option<u64>,
    pub user_time: Option<u64>,
    /// スレッドの開始時刻(FILETIME)。§8.1 で確かめた時だけ Some
    pub create_time: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawProcess {
    pub id: ProcId,
    pub parent_pid: u32,
    /// SPI の ImageName。PID 0 は空で返る(FR-1)
    pub image_name: String,
    pub session_id: u32,
    pub base_priority: i32,
    pub handle_count: u32,
    pub threads: Vec<RawThread>,
    pub kernel_time: Option<u64>,
    pub user_time: Option<u64>,
    pub working_set: u64,
    pub private_working_set: Option<u64>,
    /// PrivatePageCount(文書にある)。コミットの列
    pub private_bytes: u64,
    pub io_read_bytes: Option<u64>,
    pub io_write_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct RawSnapshot {
    /// 単調な時計のミリ秒(Clock)
    pub taken_at_ms: u64,
    pub logical_cpus: u32,
    pub processes: Vec<RawProcess>,
}

/// 状態の列。強さの順は 触れない > 終了処理中 > 一時停止中 > 応答なし > 実行中(FR-6)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ProcState {
    Running,
    Suspended,
    NotResponding,
    Exiting,
    TouchFree,
}

impl ProcState {
    pub fn label(self) -> &'static str {
        match self {
            ProcState::Running => "実行中",
            ProcState::Suspended => "一時停止中",
            ProcState::NotResponding => "応答なし",
            ProcState::Exiting => "終了処理中",
            ProcState::TouchFree => "触れない",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X64,
    X86,
    Arm64,
    Other(u16),
}

impl Arch {
    pub fn label(self) -> String {
        match self {
            Arch::X64 => "64 ビット(x64)".into(),
            Arch::X86 => "32 ビット(x86)".into(),
            Arch::Arm64 => "ARM64".into(),
            Arch::Other(m) => format!("その他(0x{m:04X})"),
        }
    }
}

/// 取れなかった理由(§9.12 の文言に直す)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InfoError {
    /// Win32 のエラー 5。管理者でないとき
    AccessDenied,
    /// 管理者でも開けない(保護されたプロセス)
    Protected,
    Gone,
    Replaced,
    TouchFree,
    /// 公開されていない API が取れない
    Unsupported,
    Other(u32),
}

/// 新しいプロセスごとに 1 回取る値(D-19 の StaticInfo)。触れないプロセスには作らない
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaticInfo {
    pub path: Option<String>,
    pub user: Option<String>,
    pub elevated: Option<bool>,
    pub critical: Option<bool>,
    pub arch: Option<Arch>,
    pub error: Option<InfoError>,
}

#[derive(Clone, Debug)]
pub struct ProcessRow {
    pub id: ProcId,
    /// 親(開始時刻で使い回しを除いた後)。親がいない・使い回しなら None(FR-2・E-6)
    pub parent: Option<ProcId>,
    pub parent_pid: u32,
    pub name: String,
    pub state: ProcState,
    pub cpu_pct: Option<f32>,
    /// 「メモリ」の列。既定はプライベート作業セット、取れなければコミット(D-10)
    pub mem: Option<u64>,
    pub working_set: u64,
    pub commit: u64,
    pub io_read_bps: Option<f64>,
    pub io_write_bps: Option<f64>,
    pub gpu_pct: Option<f32>,
    pub threads: u32,
    pub handles: u32,
    pub base_priority: i32,
    pub session_id: u32,
    pub touch_free: bool,
    pub has_window: bool,
    /// 触れないプロセスは常に None
    pub static_info: Option<Arc<StaticInfo>>,
    pub is_new: u8,
    pub is_gone: u8,
    /// 差を取るための累計(KernelTime + UserTime、100 ナノ秒)
    pub cpu_time: Option<u64>,
    pub io_read_total: Option<u64>,
    pub io_write_total: Option<u64>,
}

impl ProcessRow {
    pub fn path(&self) -> Option<&str> {
        self.static_info.as_ref().and_then(|s| s.path.as_deref())
    }
    pub fn user(&self) -> Option<&str> {
        self.static_info.as_ref().and_then(|s| s.user.as_deref())
    }
    /// 中核・System の行(PID 0・4)
    pub fn is_idle_or_system(&self) -> bool {
        self.id.pid == 0 || self.id.pid == 4
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    pub processes: u32,
    pub threads: u32,
    pub handles: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemSample {
    pub total: u64,
    pub available: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    /// システムのキャッシュ(GetPerformanceInfo の SystemCache)。0 は取れない
    pub cached: u64,
    /// カーネルのページ プールと非ページ プール。0 は取れない
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
}

impl MemSample {
    pub fn in_use(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
    pub fn used_pct(&self) -> Option<f32> {
        (self.total > 0).then(|| self.in_use() as f32 * 100.0 / self.total as f32)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskSample {
    /// PDH のインスタンス名(例: 「0 C:」)をそのまま
    pub name: String,
    pub active_pct: f32,
    pub read_bps: f64,
    pub write_bps: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetSample {
    /// MIB_IF_ROW2 の Alias
    pub name: String,
    pub send_bps: f64,
    pub recv_bps: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuSample {
    /// luid の文字列(例: 「luid_0x00000000_0x0000D1A3」)
    pub luid: String,
    /// 表示の名前(アダプター名が取れなければ「GPU <番号>」)
    pub name: String,
    pub util_pct: f32,
    pub dedicated_bytes: Option<u64>,
    pub shared_bytes: Option<u64>,
}

/// パフォーマンスのタブとトレイに出す全体の値。取れない値は None(E-11)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerfSample {
    pub cpu_total: Option<f32>,
    pub cpu_per_core: Vec<f32>,
    pub mem: Option<MemSample>,
    pub disks: Option<Vec<DiskSample>>,
    pub nets: Option<Vec<NetSample>>,
    pub gpus: Option<Vec<GpuSample>>,
    pub uptime_ms: u64,
}

/// PID ごとの GPU 使用率(D-12)
pub type GpuByPid = HashMap<u32, f32>;

/// 窓の一覧から作る PID の集合(D-32)
#[derive(Clone, Debug, Default)]
pub struct WindowInfo {
    /// 見えているトップレベルの窓を 1 つ以上持つ PID
    pub visible: HashSet<u32>,
    /// 見えている窓のどれかが IsHungAppWindow で真の PID
    pub hung: HashSet<u32>,
}

/// StaticInfo の入れ物。Collector が Worker の結果を足していく
pub type StaticCache = HashMap<ProcId, Arc<StaticInfo>>;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub taken_at_ms: u64,
    pub logical_cpus: u32,
    pub rows: Vec<ProcessRow>,
    pub totals: Totals,
    pub perf: PerfSample,
    /// 触れないプロセスの識別子(関門と画面が同じ物を見る。FR-22)
    pub touch_free: HashSet<ProcId>,
    /// プロセスごとの値を作らなかった周(最小化中。D-36)なら true
    pub rows_stale: bool,
}

impl Snapshot {
    pub fn find(&self, id: ProcId) -> Option<&ProcessRow> {
        self.rows.iter().find(|r| r.id == id)
    }
    pub fn find_pid(&self, pid: u32) -> Option<&ProcessRow> {
        self.rows.iter().find(|r| r.id.pid == pid && r.is_gone == 0)
    }
}

/// 単調な時計。テストで差し替える
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// 列(§9.2)。config.toml には snake_case の名前で書く
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Column {
    Name,
    Pid,
    State,
    Cpu,
    Mem,
    WorkingSet,
    Commit,
    IoRead,
    IoWrite,
    Gpu,
    Threads,
    Handles,
    User,
    Priority,
    Session,
    StartTime,
    Path,
    Signature,
    ParentPid,
}

impl Column {
    pub const ALL: [Column; 19] = [
        Column::Name,
        Column::Pid,
        Column::State,
        Column::Cpu,
        Column::Mem,
        Column::WorkingSet,
        Column::Commit,
        Column::IoRead,
        Column::IoWrite,
        Column::Gpu,
        Column::Threads,
        Column::Handles,
        Column::User,
        Column::Priority,
        Column::Session,
        Column::StartTime,
        Column::Path,
        Column::Signature,
        Column::ParentPid,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Pid => "pid",
            Column::State => "state",
            Column::Cpu => "cpu",
            Column::Mem => "mem",
            Column::WorkingSet => "working_set",
            Column::Commit => "commit",
            Column::IoRead => "io_read",
            Column::IoWrite => "io_write",
            Column::Gpu => "gpu",
            Column::Threads => "threads",
            Column::Handles => "handles",
            Column::User => "user",
            Column::Priority => "priority",
            Column::Session => "session",
            Column::StartTime => "start_time",
            Column::Path => "path",
            Column::Signature => "signature",
            Column::ParentPid => "parent_pid",
        }
    }

    pub fn from_key(key: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.key() == key)
    }

    /// 見出しの文字。「メモリ」はプライベート作業セットが取れないとき「コミット」と同じ意味になる(D-10)
    pub fn title(self) -> &'static str {
        match self {
            Column::Name => "名前",
            Column::Pid => "PID",
            Column::State => "状態",
            Column::Cpu => "CPU",
            Column::Mem => "メモリ",
            Column::WorkingSet => "作業セット",
            Column::Commit => "コミット",
            Column::IoRead => "I/O 読み取り",
            Column::IoWrite => "I/O 書き込み",
            Column::Gpu => "GPU",
            Column::Threads => "スレッド",
            Column::Handles => "ハンドル",
            Column::User => "ユーザー",
            Column::Priority => "優先度",
            Column::Session => "セッション",
            Column::StartTime => "開始時刻",
            Column::Path => "パス",
            Column::Signature => "署名",
            Column::ParentPid => "親 PID",
        }
    }

    /// 数字の列は右寄せ
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Column::Pid
                | Column::Cpu
                | Column::Mem
                | Column::WorkingSet
                | Column::Commit
                | Column::IoRead
                | Column::IoWrite
                | Column::Gpu
                | Column::Threads
                | Column::Handles
                | Column::Session
                | Column::ParentPid
        )
    }

    pub fn default_width(self) -> f32 {
        match self {
            Column::Name => 240.0,
            Column::Path => 320.0,
            Column::User | Column::Signature => 140.0,
            Column::StartTime => 150.0,
            Column::State | Column::Priority => 90.0,
            _ => 80.0,
        }
    }
}

/// 優先度の 5 つ(D-16。リアルタイムは出さない)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Priority {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
}

impl Priority {
    pub const ALL: [Priority; 5] = [
        Priority::Idle,
        Priority::BelowNormal,
        Priority::Normal,
        Priority::AboveNormal,
        Priority::High,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Priority::Idle => "低",
            Priority::BelowNormal => "通常以下",
            Priority::Normal => "通常",
            Priority::AboveNormal => "通常以上",
            Priority::High => "高",
        }
    }
    /// SPI の BasePriority から(FR-12)。4→低、6→通常以下、8→通常、10→通常以上、13→高
    pub fn from_base(base: i32) -> Option<Priority> {
        match base {
            4 => Some(Priority::Idle),
            6 => Some(Priority::BelowNormal),
            8 => Some(Priority::Normal),
            10 => Some(Priority::AboveNormal),
            13 => Some(Priority::High),
            _ => None,
        }
    }
    pub fn base_label(base: i32) -> String {
        Priority::from_base(base).map_or_else(|| base.to_string(), |p| p.label().to_string())
    }
}
