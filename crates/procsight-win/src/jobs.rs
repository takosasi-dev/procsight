//! 画面(procsight-app)と裏のスレッドの間でやり取りする型(INV-9)。画面は Job を送り、JobResult を受けるだけ。
//! 文言の正は §9.12 と画面設計書 §10。ここで決めた型は docs/INTERFACES.md の契約。

use crate::gate::GateError;
use procsight_core::rules::Refusal;
use procsight_core::startup::{StartupItem, StartupLocation};
use procsight_core::{Arch, InfoError, Priority, ProcId};
use std::sync::{Arc, Mutex};

/// 操作が失敗した理由。`message` で §9.12 の文にする
#[derive(Clone, Debug, PartialEq)]
pub enum OpError {
    Gate(GateError),
    Refused(Refusal),
    /// Win32 のエラー番号
    Win32(u32),
    /// 公開されていない API が取れない(D-6)
    Unsupported,
    /// サービスの状態の変化が 30 秒で終わらない(E-31)
    ServiceTimeout,
    /// UAC を断った(E-35。エラー 1223)
    Cancelled,
    /// 置き換えの条件(D-39 など)を満たさない
    Blocked(&'static str),
}

impl OpError {
    pub fn from_code(code: u32) -> OpError {
        match code {
            1223 => OpError::Cancelled,
            c => OpError::Win32(c),
        }
    }

    /// 権限が足りないための失敗か(FR-20 の「管理者で再起動」を出すか)
    pub fn is_access_denied(&self) -> bool {
        matches!(self, OpError::Gate(GateError::AccessDenied) | OpError::Win32(5))
    }

    /// §9.12 の文。`elevated` は ProcSight が昇格しているか
    pub fn message(&self, elevated: bool) -> String {
        let denied = |elevated: bool| {
            if elevated {
                "このプロセスは保護されているため、操作できません".to_string()
            } else {
                "権限が足りません。管理者で再起動すると操作できることがあります".to_string()
            }
        };
        match self {
            OpError::Gate(GateError::TouchFree) => "触れないプロセスに登録されています".into(),
            OpError::Gate(GateError::Replaced) => "選んだ後にプロセスが入れ替わりました。一覧を見直してください".into(),
            OpError::Gate(GateError::Gone) | OpError::Win32(87) => "すでに終了しています".into(),
            OpError::Gate(GateError::AccessDenied) => denied(false),
            OpError::Gate(GateError::Protected) => denied(true),
            OpError::Win32(5) => denied(elevated),
            OpError::Gate(GateError::Other(c)) | OpError::Win32(c) => {
                format!("失敗しました(エラー {c}: {})", crate::util::error_text(*c))
            }
            OpError::Refused(r) => r.message().into(),
            OpError::Unsupported => "この Windows では、この機能を使えません".into(),
            OpError::ServiceTimeout => "サービスが 30 秒以内に応答しませんでした。状態を見直してください".into(),
            OpError::Cancelled => "管理者への切り替えをやめました".into(),
            OpError::Blocked(m) => (*m).into(),
        }
    }
}

impl From<GateError> for OpError {
    fn from(e: GateError) -> Self {
        OpError::Gate(e)
    }
}

impl From<InfoError> for OpError {
    fn from(e: InfoError) -> Self {
        match e {
            InfoError::AccessDenied => OpError::Gate(GateError::AccessDenied),
            InfoError::Protected => OpError::Gate(GateError::Protected),
            InfoError::Gone => OpError::Gate(GateError::Gone),
            InfoError::Replaced => OpError::Gate(GateError::Replaced),
            InfoError::TouchFree => OpError::Gate(GateError::TouchFree),
            InfoError::Unsupported => OpError::Unsupported,
            InfoError::Other(c) => OpError::Win32(c),
        }
    }
}

/// 操作の対象(識別子と、帯に出す名前)
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: ProcId,
    pub name: String,
}

// ---- 詳細パネル(FR-30〜FR-34) ----

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralInfo {
    pub path: Result<String, OpError>,
    pub command_line: Result<String, OpError>,
    pub user: Result<String, OpError>,
    pub elevated: Result<bool, OpError>,
    /// 「低」「中」「高」「システム」など。値は SID の RID から
    pub integrity: Result<String, OpError>,
    pub arch: Result<Arch, OpError>,
    /// KernelTime + UserTime(100 ナノ秒)
    pub cpu_time: Result<u64, OpError>,
    /// 効率モード(読めたとき。FR-14)
    pub efficiency: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleRow {
    pub name: String,
    pub path: String,
    pub base: u64,
    pub size: u32,
}

/// ハンドルの名前の状態(D-22・E-22)
#[derive(Clone, Debug, PartialEq)]
pub enum HandleName {
    Pending,
    /// 名前の無いハンドル
    None,
    Name(String),
    /// 500 ミリ秒で返らなかった
    TimedOut,
    /// 問い合わせを打ち切った種類(File で捨てたワーカーが 3 つになった後)
    Skipped,
    Failed(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HandleRow {
    pub value: u64,
    pub type_name: String,
    pub name: HandleName,
    pub access: u32,
}

/// ハンドルの一覧は名前を後から埋めるので、画面と HandleNameWorker で共有する
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandleTable {
    pub rows: Vec<HandleRow>,
    pub names_done: usize,
    pub names_total: usize,
    pub finished: bool,
}

pub type SharedHandleTable = Arc<Mutex<HandleTable>>;

#[derive(Clone, Debug, PartialEq)]
pub enum SigStatus {
    Embedded,
    Catalog,
    Unsigned,
    /// 検証に失敗(理由)
    Failed(String),
}

impl SigStatus {
    pub fn label(&self) -> String {
        match self {
            SigStatus::Embedded => "署名あり(埋め込み)".into(),
            SigStatus::Catalog => "署名あり(カタログ)".into(),
            SigStatus::Unsigned => "署名なし".into(),
            SigStatus::Failed(r) => format!("検証に失敗({r})"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub status: SigStatus,
    pub signer: Option<String>,
    /// 確かめた時刻(FILETIME)
    pub checked_at: u64,
}

// ---- ファイルを掴んでいるプロセス(FR-35・D-25) ----

#[derive(Clone, Debug, PartialEq)]
pub struct LockingProcess {
    pub pid: u32,
    /// RM_UNIQUE_PROCESS の開始時刻(FILETIME)
    pub create_time: u64,
    pub app_name: String,
    /// 「アプリ」「サービス」「エクスプローラー」「コンソール」「重要」「その他」
    pub kind: String,
    pub service_short_name: Option<String>,
}

// ---- サービス(FR-41〜FR-44・D-34) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
}

impl ServiceState {
    pub fn label(self) -> &'static str {
        match self {
            ServiceState::Stopped => "停止",
            ServiceState::StartPending | ServiceState::ContinuePending => "開始中",
            ServiceState::StopPending | ServiceState::PausePending => "停止中",
            ServiceState::Running => "実行中",
            ServiceState::Paused => "一時停止",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartType {
    Auto,
    AutoDelayed,
    Manual,
    Disabled,
    /// ドライバ用の値(SERVICE_WIN32 では普通は出ない)
    Boot,
    System,
}

impl StartType {
    pub const CHOICES: [StartType; 4] = [StartType::Auto, StartType::AutoDelayed, StartType::Manual, StartType::Disabled];
    pub fn label(self) -> &'static str {
        match self {
            StartType::Auto => "自動",
            StartType::AutoDelayed => "自動(遅延開始)",
            StartType::Manual => "手動",
            StartType::Disabled => "無効",
            StartType::Boot => "ブート",
            StartType::System => "システム",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceRow {
    pub name: String,
    pub display_name: String,
    pub state: ServiceState,
    pub start_type: Option<StartType>,
    pub pid: u32,
    pub description: String,
    pub account: String,
    pub accepts_stop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

// ---- 置き換え(FR-53〜FR-56) ----

#[derive(Clone, Debug, PartialEq)]
pub struct ReplaceInfo {
    /// 普段のビューの Debugger の値
    pub debugger: Option<String>,
    /// WOW6432Node の側の値(読むだけ。D-41)
    pub debugger_wow64: Option<String>,
    /// 今の ProcSight.exe の実体のパス
    pub self_final_path: String,
    /// オンにできないとき、その理由(D-39)
    pub block: Option<procsight_core::taskmgr::ReplaceBlock>,
}

// ---- Job と結果 ----

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// 終了(FR-10・FR-11・FR-18)。並べた順に終える(ツリーは葉から)。Worker が直前に rules::check_terminate を通す
    Terminate { targets: Vec<Target>, tree: bool },
    SetPriority { target: Target, priority: Priority },
    GetAffinity { target: Target },
    SetAffinity { target: Target, mask: u64 },
    SetEfficiency { target: Target, on: bool },
    Suspend { target: Target },
    Resume { target: Target },
    /// 一時停止の記録から再開する(E-12・FR-51)
    ResumeMany { targets: Vec<Target> },
    BringToFront { target: Target },
    OpenLocation { path: String },
    ShowProperties { path: String },
    /// FR-23・FR-55。置き換えがオンなら D-40 の手順
    OpenTaskManager,
    General { id: ProcId },
    /// スレッドの一覧(D-20)。SPI から作るのでハンドルを開かない
    Threads { id: ProcId },
    Modules { id: ProcId },
    Handles { id: ProcId },
    /// D-23。パス・大きさ・更新時刻を鍵に覚え、同じなら取り直さない
    Signature { path: String },
    FindLocking { paths: Vec<String> },
    LoadStartup,
    SetStartup { name: String, location: StartupLocation, enabled: bool },
    LoadServices,
    /// 止めるサービスに依存して動いているサービス(D-34。確認の窓に出す)
    ServiceDependents { name: String },
    /// `confirmed` は確認の窓(D5)に出した依存のサービスの名前。止める直前に取り直した一覧がこの中に収まらなければ止めない(INV-5)
    ServiceControl { name: String, display_name: String, action: ServiceAction, confirmed: Vec<String> },
    SetStartType { name: String, display_name: String, start_type: StartType },
    /// 16×16 の RGBA(D-33)
    Icon { path: String },
    /// D-18。成功したら app は自分を終える
    RestartElevated,
    ReplaceStatus,
    ReplaceEnable,
    /// `previous` は config.toml の taskmgr_replace.previous_debugger(D-41)
    ReplaceDisable { previous: String },
    /// E-45 の「もう一度」。D-40 で戻せなかった値(ProcSight を指す物だけ)を書き戻す
    ReplaceRestore { value: String },
    /// ファイルを選ぶ窓(IFileOpenDialog、複数選択。FR-35)
    PickFiles,
}

#[derive(Clone, Debug)]
pub enum JobResult {
    /// 終了の結果。1 つずつの成功・失敗
    Terminated { results: Vec<(Target, Result<(), OpError>)>, tree: bool },
    /// 優先度・割り当て・効率モード・一時停止・再開・前面・場所・プロパティの結果
    Op { job: Job, result: Result<(), OpError> },
    Affinity { target: Target, result: Result<(u64, u64), OpError> },
    General { id: ProcId, info: Result<GeneralInfo, OpError> },
    Threads { id: ProcId, result: Result<Vec<procsight_core::RawThread>, OpError> },
    Modules { id: ProcId, result: Result<Vec<ModuleRow>, OpError> },
    /// 名前は HandleNameWorker が後から埋める
    Handles { id: ProcId, result: Result<SharedHandleTable, OpError> },
    Signature { path: String, result: Signature },
    Locking { paths: Vec<String>, result: Result<Vec<LockingProcess>, OpError> },
    Startup { result: Result<Vec<StartupItem>, OpError> },
    StartupSet { name: String, enabled: bool, result: Result<(), OpError> },
    Services { result: Result<Vec<ServiceRow>, OpError> },
    ServiceDependents { name: String, result: Result<Vec<(String, String)>, OpError> },
    ServiceDone { display_name: String, action: ServiceAction, result: Result<(), OpError> },
    StartTypeSet { display_name: String, start_type: StartType, result: Result<(), OpError> },
    Icon { path: String, rgba: Option<Vec<u8>> },
    RestartElevated { result: Result<(), OpError> },
    Replace { info: ReplaceInfo },
    ReplaceChanged { enabled: bool, previous: String, result: Result<(), OpError> },
    /// 置き換えを外して本物のタスク マネージャーを開いた後、値を戻せなかった(E-45)。`saved` は戻す値
    ReplaceRestoreFailed { saved: String },
    ReplaceRestored { result: Result<(), OpError> },
    PickedFiles { paths: Vec<String> },
}
