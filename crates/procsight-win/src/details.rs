//! 詳細パネルの取得(担当 B。FR-30〜FR-35)。全般(D-19)・DLL(D-21)・ハンドル(D-22)・署名(D-23)・
//! ファイルを掴んでいるプロセス(D-25)。ハンドルの名前は HandleNameWorker(専用のスレッド)が後から埋める。

use crate::elevation::{integrity_label, open_token, token_elevated, token_integrity_rid, token_user};
use crate::gate::{GateError, OwnedHandle, ProcessGate};
use crate::jobs::{
    GeneralInfo, HandleName, HandleRow, HandleTable, LockingProcess, ModuleRow, OpError, SharedHandleTable, SigStatus,
    Signature,
};
use crate::util::{from_wide, pcwstr, wide, win32_code};
use procsight_core::{Arch, ProcId};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{FILETIME, HANDLE, HWND};
use windows::Win32::System::Threading::{
    GetProcessInformation, GetProcessTimes, IsWow64Process2, PROCESS_DUP_HANDLE, PROCESS_NAME_WIN32,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, ProcessPowerThrottling,
    QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

const STATUS_ACCESS_DENIED: i32 = 0xC000_0022u32 as i32;

// ---- 全般(D-19) ----

pub fn general(gate: &ProcessGate, id: ProcId) -> Result<GeneralInfo, OpError> {
    let p = gate.open(id, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let h = p.raw();
    let token = open_token(h).map_err(OpError::from_code);
    Ok(GeneralInfo {
        path: image_path(h),
        command_line: crate::nt::cmdline::query(h).map_err(|st| match st {
            STATUS_ACCESS_DENIED => OpError::Win32(5),
            _ => OpError::Unsupported,
        }),
        user: with_token(&token, token_user),
        elevated: with_token(&token, token_elevated),
        integrity: with_token(&token, |t| token_integrity_rid(t).map(|r| integrity_label(r).to_string())),
        arch: arch_of(h),
        cpu_time: cpu_time(h),
        efficiency: efficiency(h),
    })
}

fn with_token<T>(
    token: &Result<crate::elevation::Token, OpError>,
    f: impl Fn(&crate::elevation::Token) -> Result<T, u32>,
) -> Result<T, OpError> {
    match token {
        Ok(t) => f(t).map_err(OpError::from_code),
        Err(e) => Err(e.clone()),
    }
}

fn image_path(h: HANDLE) -> Result<String, OpError> {
    let mut buf = vec![0u16; 1024];
    let mut n = buf.len() as u32;
    // SAFETY: buf の長さ(文字数)を n で渡し、書かれた n 文字だけを読む
    unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n) }
        .map_err(|e| OpError::from_code(win32_code(&e)))?;
    Ok(String::from_utf16_lossy(&buf[..n as usize]))
}

fn arch_of(h: HANDLE) -> Result<Arch, OpError> {
    use windows::Win32::System::SystemInformation::{
        IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
        IMAGE_FILE_MACHINE_UNKNOWN,
    };
    let (mut process, mut native) = (IMAGE_FILE_MACHINE::default(), IMAGE_FILE_MACHINE::default());
    // SAFETY: h は関門で開いたハンドル。2 つの出力はこの関数の中の変数
    unsafe { IsWow64Process2(h, &mut process, Some(&mut native)) }.map_err(|e| OpError::from_code(win32_code(&e)))?;
    // WOW64 でなければ process は UNKNOWN で、そのプロセスは OS と同じ種類
    let m = if process == IMAGE_FILE_MACHINE_UNKNOWN { native } else { process };
    Ok(match m {
        IMAGE_FILE_MACHINE_AMD64 => Arch::X64,
        IMAGE_FILE_MACHINE_I386 => Arch::X86,
        IMAGE_FILE_MACHINE_ARM64 => Arch::Arm64,
        other => Arch::Other(other.0),
    })
}

fn cpu_time(h: HANDLE) -> Result<u64, OpError> {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: h は関門で開いたハンドル。4 つの FILETIME はこの関数の中の変数
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.map_err(|e| OpError::from_code(win32_code(&e)))?;
    Ok(crate::util::filetime_u64(k) + crate::util::filetime_u64(u))
}

/// EcoQoS(実行速度の抑制)が入っているか。読めなければ None
fn efficiency(h: HANDLE) -> Option<bool> {
    let mut st = PROCESS_POWER_THROTTLING_STATE { Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION, ..Default::default() };
    // SAFETY: st はこの関数の中の構造体で、その大きさを渡す
    unsafe {
        GetProcessInformation(
            h,
            ProcessPowerThrottling,
            (&mut st as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .ok()?;
    let bit = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
    Some(st.ControlMask & bit != 0 && st.StateMask & bit != 0)
}

// ---- DLL(D-21・E-24) ----

const ERROR_BAD_LENGTH: u32 = 24;

pub fn modules(gate: &ProcessGate, id: ProcId) -> Result<Vec<ModuleRow>, OpError> {
    for attempt in 0..4 {
        match gate.module_snapshot(id) {
            Ok(snap) => return Ok(walk_modules(&snap)),
            Err(GateError::Other(ERROR_BAD_LENGTH)) if attempt < 3 => std::thread::sleep(Duration::from_millis(30)),
            Err(GateError::Other(ERROR_BAD_LENGTH)) => break,
            Err(e) => return Err(e.into()),
        }
    }
    // E-24。「取得できません(しばらくしてもう一度)」の括弧の中
    Err(OpError::Blocked("しばらくしてもう一度"))
}

fn walk_modules(snap: &OwnedHandle) -> Vec<ModuleRow> {
    use windows::Win32::System::Diagnostics::ToolHelp::{MODULEENTRY32W, Module32FirstW, Module32NextW};
    let mut out = Vec::new();
    let mut me = MODULEENTRY32W { dwSize: size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
    // SAFETY: snap は関門が作ったスナップショットのハンドル。me はこの関数の中の構造体で dwSize を入れてある
    let mut ok = unsafe { Module32FirstW(snap.raw(), &mut me) }.is_ok();
    while ok {
        out.push(ModuleRow {
            name: from_wide(&me.szModule),
            path: from_wide(&me.szExePath),
            base: me.modBaseAddr as u64,
            size: me.modBaseSize,
        });
        // SAFETY: 同上
        ok = unsafe { Module32NextW(snap.raw(), &mut me) }.is_ok();
    }
    out
}

// ---- ハンドル(D-22・E-22) ----

/// 1 つの名前の問い合わせを待つ時間
const NAME_TIMEOUT: Duration = Duration::from_millis(500);
/// 1 回の一覧で捨てたワーカーがこの数になったら、残りの File の名前は問い合わせない
const MAX_ABANDONED: u32 = 3;

/// 複製したハンドルをワーカーのスレッドへ所有ごと渡すための包み。
/// SAFETY: 中のハンドルは DuplicateHandle で自分のプロセスに作った物で、どのスレッドからでも使え、
/// 渡した先のスレッドだけが持って閉じる(捨てたワーカーのハンドルは外から閉じない)
struct SendHandle(OwnedHandle);
// SAFETY: 上のとおり。カーネルのハンドルはスレッドに結び付かない
unsafe impl Send for SendHandle {}

/// HandleNameWorker。名前の問い合わせを 1 つずつ専用のスレッドで行う。止まったら捨てて作り直す
struct NameWorker {
    tx: Sender<SendHandle>,
    rx: Receiver<Result<Option<String>, i32>>,
}

impl NameWorker {
    fn spawn() -> NameWorker {
        let (tx, jobs) = channel::<SendHandle>();
        let (reply, rx) = channel();
        std::thread::Builder::new()
            .name("handle-name".into())
            .spawn(move || {
                // 送り手(NameWorker)が捨てられたら recv が失敗して終わる
                for h in jobs {
                    let r = crate::nt::handles::query_name(h.0.raw());
                    drop(h);
                    if reply.send(r).is_err() {
                        break;
                    }
                }
            })
            .ok();
        NameWorker { tx, rx }
    }

    /// Err(()) は時間切れか、ワーカーが居なくなった(どちらも作り直す)
    fn ask(&self, h: OwnedHandle) -> Result<Result<Option<String>, i32>, RecvTimeoutError> {
        self.tx.send(SendHandle(h)).map_err(|_| RecvTimeoutError::Disconnected)?;
        self.rx.recv_timeout(NAME_TIMEOUT)
    }
}

fn gate_code(e: GateError) -> u32 {
    match e {
        GateError::AccessDenied | GateError::Protected | GateError::TouchFree => 5,
        GateError::Gone | GateError::Replaced => 87,
        GateError::Other(c) => c,
    }
}

/// 一覧(種類・値・アクセス)を作って返し、名前は HandleNameWorker が後から埋める
pub fn handles(gate: &Arc<ProcessGate>, id: ProcId) -> Result<SharedHandleTable, OpError> {
    if gate.is_touch_free(id) {
        return Err(OpError::Gate(GateError::TouchFree));
    }
    let types = crate::nt::handles::type_names().ok_or(OpError::Unsupported)?;
    let raw = crate::nt::handles::for_pid(id.pid).map_err(|_| OpError::Unsupported)?;
    let rows: Vec<HandleRow> = raw
        .iter()
        .map(|h| HandleRow {
            value: h.value as u64,
            type_name: types.get(&h.type_index).cloned().unwrap_or_else(|| format!("#{}", h.type_index)),
            name: HandleName::Pending,
            access: h.access,
        })
        .collect();
    let total = rows.len();
    let table = Arc::new(Mutex::new(HandleTable { rows, names_done: 0, names_total: total, finished: false }));
    let (g, t) = (Arc::clone(gate), Arc::clone(&table));
    std::thread::Builder::new()
        .name("handle-names".into())
        .spawn(move || fill_names(&g, id, &t))
        .map_err(|e| OpError::Win32(e.raw_os_error().unwrap_or(0) as u32))?;
    Ok(table)
}

/// 複製しない種類。Process と Thread は名前を持たず(自分のハンドルで確かめた)、複製すると相手が持つ
/// 他のプロセス(触れないプロセスを含む)のハンドルを、関門を通さずに同じアクセス権で持つことになる(INV-1)
fn never_duplicate(type_name: &str) -> bool {
    matches!(type_name, "Process" | "Thread")
}

fn fill_names(gate: &ProcessGate, id: ProcId, table: &SharedHandleTable) {
    let work: Vec<(u64, bool, bool)> = {
        let t = table.lock().unwrap_or_else(|e| e.into_inner());
        t.rows.iter().map(|r| (r.value, r.type_name == "File", never_duplicate(&r.type_name))).collect()
    };
    let set = |i: usize, name: HandleName| {
        let mut t = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = t.rows.get_mut(i) {
            r.name = name;
        }
        t.names_done += 1;
    };
    match gate.open(id, PROCESS_DUP_HANDLE) {
        Err(e) => {
            for (i, &(_, _, skip)) in work.iter().enumerate() {
                set(i, if skip { HandleName::None } else { HandleName::Failed(gate_code(e)) });
            }
        }
        Ok(source) => {
            let mut worker = NameWorker::spawn();
            let mut abandoned = 0u32;
            for (i, &(value, is_file, skip)) in work.iter().enumerate() {
                if skip {
                    set(i, HandleName::None);
                    continue;
                }
                if is_file && abandoned >= MAX_ABANDONED {
                    set(i, HandleName::Skipped);
                    continue;
                }
                let name = match gate.duplicate_from(&source, value as usize) {
                    // EtwRegistration などは複製できない(ERROR_NOT_SUPPORTED)。名前の無いハンドルとして出す
                    Err(GateError::Other(50)) => HandleName::None,
                    Err(e) => HandleName::Failed(gate_code(e)),
                    // 一覧を取った後に相手がハンドルの値を使い直し、Process・Thread になっていたら、すぐ閉じて名前を聞かない
                    // (他のプロセスへのハンドルを持ち続けない。INV-1)
                    Ok(h) if crate::nt::handles::query_type(h.raw()).is_none_or(|t| never_duplicate(&t)) => {
                        drop(h);
                        HandleName::None
                    }
                    Ok(h) => match worker.ask(h) {
                        Ok(Ok(Some(s))) => HandleName::Name(s),
                        Ok(Ok(None)) => HandleName::None,
                        Ok(Err(st)) => HandleName::Failed(st as u32),
                        Err(RecvTimeoutError::Timeout) => {
                            // 止まったスレッドは殺さずに放す(D-22)
                            abandoned += 1;
                            worker = NameWorker::spawn();
                            HandleName::TimedOut
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            worker = NameWorker::spawn();
                            HandleName::Failed(0)
                        }
                    },
                };
                set(i, name);
            }
        }
    }
    table.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
}

// ---- 署名(D-23・E-25) ----

const TRUST_E_NOSIGNATURE: u32 = 0x800B_0100;
const TRUST_E_SUBJECT_FORM_UNKNOWN: u32 = 0x800B_0003;
const TRUST_E_PROVIDER_UNKNOWN: u32 = 0x800B_0001;

type SigKey = (String, u64, u64);

/// 署名の結果を (パス、大きさ、更新時刻) を鍵に覚える(D-23)
pub struct SignatureCache {
    map: Mutex<HashMap<SigKey, Signature>>,
}

impl SignatureCache {
    pub fn new() -> SignatureCache {
        SignatureCache { map: Mutex::new(HashMap::new()) }
    }

    pub fn check(&self, path: &str) -> Signature {
        use std::os::windows::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file());
        let key = match &meta {
            Some(m) => (path.to_lowercase(), m.len(), m.last_write_time()),
            None => (path.to_lowercase(), u64::MAX, u64::MAX),
        };
        if let Some(s) = self.map.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return s.clone();
        }
        let (status, signer) = match meta {
            None => (SigStatus::Failed("ファイルが見つかりません".into()), None),
            Some(_) => verify(path),
        };
        let sig = Signature { status, signer, checked_at: crate::util::now_filetime() };
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(key, sig.clone());
        sig
    }
}

impl Default for SignatureCache {
    fn default() -> Self {
        Self::new()
    }
}

fn trust_reason(hr: u32) -> String {
    if hr == 0x8007_0002 || hr == 0x8007_0003 {
        return "ファイルが見つかりません".into();
    }
    let t = crate::util::error_text(hr);
    if t.is_empty() { format!("0x{hr:08X}") } else { format!("{t} 0x{hr:08X}") }
}

/// 埋め込み → カタログの順に確かめる
fn verify(path: &str) -> (SigStatus, Option<String>) {
    let wpath = wide(path);
    match verify_embedded(&wpath) {
        (0, signer) => (SigStatus::Embedded, signer),
        (TRUST_E_NOSIGNATURE | TRUST_E_SUBJECT_FORM_UNKNOWN | TRUST_E_PROVIDER_UNKNOWN, _) => {
            match verify_catalog(path, &wpath) {
                Some((0, signer)) => (SigStatus::Catalog, signer),
                Some((hr, _)) => (SigStatus::Failed(trust_reason(hr)), None),
                None => (SigStatus::Unsigned, None),
            }
        }
        (hr, _) => (SigStatus::Failed(trust_reason(hr)), None),
    }
}

/// WinVerifyTrust を VERIFY で呼び、成功なら署名者を取ってから CLOSE する。戻り値は (結果, 署名者)
fn run_trust(data: &mut windows::Win32::Security::WinTrust::WINTRUST_DATA) -> (u32, Option<String>) {
    use windows::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
    };
    data.cbStruct = size_of::<windows::Win32::Security::WinTrust::WINTRUST_DATA>() as u32;
    data.dwUIChoice = WTD_UI_NONE;
    // INV-3: 失効の確認をせず、ネットから取りに行かない
    data.fdwRevocationChecks = WTD_REVOKE_NONE;
    data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE;
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let no_ui = HWND(-1isize as *mut core::ffi::c_void); // INVALID_HANDLE_VALUE: 窓を出さない
    // SAFETY: data はこの呼び出しの間生きている構造体で、中の指す先も呼ぶ側の変数
    let r = unsafe { WinVerifyTrust(no_ui, &mut action, data as *mut _ as *mut core::ffi::c_void) } as u32;
    let signer = if r == 0 { signer_from_state(data.hWVTStateData) } else { None };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: 同じ構造体で状態を閉じる(署名者は上で取り終えた)
    unsafe { WinVerifyTrust(no_ui, &mut action, data as *mut _ as *mut core::ffi::c_void) };
    (r, signer)
}

// windows crate の WTHelper* は機能 Win32_Security_Cryptography_Sip が要り、Cargo.toml は統合担当の物なので、
// wintrust.dll から直接読む(raw-dylib。導入ライブラリは要らない)。機能が足されたら windows crate の物に替えてよい
#[link(name = "wintrust", kind = "raw-dylib")]
unsafe extern "system" {
    fn WTHelperProvDataFromStateData(hstatedata: HANDLE) -> *mut core::ffi::c_void;
    fn WTHelperGetProvSignerFromChain(
        pprovdata: *mut core::ffi::c_void,
        idxsigner: u32,
        fcountersigner: windows::core::BOOL,
        idxcountersigner: u32,
    ) -> *mut windows::Win32::Security::WinTrust::CRYPT_PROVIDER_SGNR;
}

fn signer_from_state(state: HANDLE) -> Option<String> {
    use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
    // SAFETY: state は VERIFY が返した状態で、CLOSE の前に読む。各ポインタは null を確かめてから辿る
    unsafe {
        let pd = WTHelperProvDataFromStateData(state);
        if pd.is_null() {
            return None;
        }
        let sg = WTHelperGetProvSignerFromChain(pd, 0, false.into(), 0);
        if sg.is_null() || (*sg).csCertChain == 0 || (*sg).pasCertChain.is_null() {
            return None;
        }
        let cert = (*(*sg).pasCertChain).pCert;
        if cert.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 512];
        let n = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf));
        (n > 1).then(|| from_wide(&buf))
    }
}

fn verify_embedded(wpath: &[u16]) -> (u32, Option<String>) {
    use windows::Win32::Security::WinTrust::{WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE};
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: pcwstr(wpath),
        ..Default::default()
    };
    let mut data =
        WINTRUST_DATA { dwUnionChoice: WTD_CHOICE_FILE, Anonymous: WINTRUST_DATA_0 { pFile: &mut file }, ..Default::default() };
    run_trust(&mut data)
}

/// カタログで確かめる。カタログに無ければ None
fn verify_catalog(path: &str, wpath: &[u16]) -> Option<(u32, Option<String>)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Security::Cryptography::Catalog::{
        CATALOG_INFO, CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
        CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext, CryptCATAdminReleaseContext,
        CryptCATCatalogInfoFromContext,
    };
    use windows::Win32::Security::WinTrust::{WINTRUST_CATALOG_INFO, WINTRUST_DATA, WINTRUST_DATA_0, WTD_CHOICE_CATALOG};
    let file = std::fs::File::open(path).ok()?;
    let hfile = HANDLE(file.as_raw_handle());
    // 新しいカタログは SHA256、古い物は SHA1
    for alg in ["SHA256", "SHA1"] {
        let walg = wide(alg);
        let mut admin = 0isize;
        // SAFETY: admin はこの関数の中の変数。アルゴリズム名は 0 で終わる
        if unsafe { CryptCATAdminAcquireContext2(&mut admin, None, pcwstr(&walg), None, None) }.is_err() {
            continue;
        }
        let result = (|| {
            let mut n = 0u32;
            // SAFETY: 大きさを聞いてから、その大きさのバッファに書かせる。hfile は上で開いたファイル
            unsafe { CryptCATAdminCalcHashFromFileHandle2(admin, hfile, &mut n, None, None) }.ok()?;
            let mut hash = vec![0u8; n as usize];
            // SAFETY: 同上
            unsafe { CryptCATAdminCalcHashFromFileHandle2(admin, hfile, &mut n, Some(hash.as_mut_ptr()), None) }.ok()?;
            // SAFETY: hash は上で作った配列
            let cat = unsafe { CryptCATAdminEnumCatalogFromHash(admin, &hash, None, None) };
            if cat == 0 {
                return None;
            }
            let mut info = CATALOG_INFO { cbStruct: size_of::<CATALOG_INFO>() as u32, ..Default::default() };
            // SAFETY: cat は上で取ったカタログの文脈。info はこの関数の中の構造体
            let got = unsafe { CryptCATCatalogInfoFromContext(cat, &mut info, 0) }.is_ok();
            let out = got.then(|| {
                let tag = wide(&hash.iter().map(|b| format!("{b:02X}")).collect::<String>());
                let mut ci = WINTRUST_CATALOG_INFO {
                    cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
                    pcwszCatalogFilePath: windows::core::PCWSTR(info.wszCatalogFile.as_ptr()),
                    pcwszMemberTag: pcwstr(&tag),
                    pcwszMemberFilePath: pcwstr(wpath),
                    hMemberFile: hfile,
                    pbCalculatedFileHash: hash.as_mut_ptr(),
                    cbCalculatedFileHash: hash.len() as u32,
                    hCatAdmin: admin,
                    ..Default::default()
                };
                let mut data = WINTRUST_DATA {
                    dwUnionChoice: WTD_CHOICE_CATALOG,
                    Anonymous: WINTRUST_DATA_0 { pCatalog: &mut ci },
                    ..Default::default()
                };
                run_trust(&mut data)
            });
            // SAFETY: 上で取ったカタログの文脈を 1 回だけ返す
            unsafe {
                let _ = CryptCATAdminReleaseCatalogContext(admin, cat, 0);
            }
            out
        })();
        // SAFETY: 上で取った文脈を 1 回だけ返す
        unsafe {
            let _ = CryptCATAdminReleaseContext(admin, 0);
        }
        if result.is_some() {
            return result;
        }
    }
    None
}

// ---- ファイルを掴んでいるプロセス(D-25・E-26・E-27) ----

pub const MAX_LOCKING_FILES: usize = 100;

/// 渡せない物を断る(E-26)。フォルダと 101 個以上
fn check_locking_paths(paths: &[String]) -> Result<(), OpError> {
    if paths.len() > MAX_LOCKING_FILES {
        return Err(OpError::Blocked("一度に 100 ファイルまでです"));
    }
    if paths.iter().any(|p| std::path::Path::new(p).is_dir()) {
        return Err(OpError::Blocked("フォルダは調べられません。中のファイルを選んでください"));
    }
    Ok(())
}

fn app_kind(t: windows::Win32::System::RestartManager::RM_APP_TYPE) -> &'static str {
    // RM_APP_TYPE: RmMainWindow 1、RmOtherWindow 2、RmService 3、RmExplorer 4、RmConsole 5、RmCritical 1000
    match t.0 {
        1 | 2 => "アプリ",
        3 => "サービス",
        4 => "エクスプローラー",
        5 => "コンソール",
        1000 => "重要",
        _ => "その他",
    }
}

/// Restart Manager で探す(D-25)。フォルダと 101 個以上は断る(E-26)。触れないプロセスがいる間は呼ばない(呼ぶ側が確かめる)
pub fn find_locking(paths: &[String]) -> Result<Vec<LockingProcess>, OpError> {
    use windows::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources, RmStartSession,
    };
    check_locking_paths(paths)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut session = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: session と key はこの関数の中の変数(key は CCH_RM_SESSION_KEY + 1 文字)
    let rc = unsafe { RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) };
    if rc.0 != 0 {
        return Err(OpError::from_code(rc.0));
    }
    let result = (|| {
        let wides: Vec<Vec<u16>> = paths.iter().map(|p| wide(p)).collect();
        let ptrs: Vec<windows::core::PCWSTR> = wides.iter().map(|w| pcwstr(w)).collect();
        // SAFETY: ptrs の各要素は wides の 0 で終わる配列を指し、この呼び出しの間生きている
        let rc = unsafe { RmRegisterResources(session, Some(&ptrs), None, None) };
        if rc.0 != 0 {
            return Err(OpError::from_code(rc.0));
        }
        let mut infos: Vec<RM_PROCESS_INFO> = vec![RM_PROCESS_INFO::default(); 16];
        for _ in 0..5 {
            let (mut needed, mut n, mut reasons) = (0u32, infos.len() as u32, 0u32);
            // SAFETY: infos の長さを n で渡し、書かれた n 件だけを読む
            let rc = unsafe { RmGetList(session, &mut needed, &mut n, Some(infos.as_mut_ptr()), &mut reasons) };
            match rc.0 {
                0 => {
                    infos.truncate(n as usize);
                    return Ok(infos
                        .iter()
                        .map(|i| LockingProcess {
                            pid: i.Process.dwProcessId,
                            create_time: crate::util::filetime_u64(i.Process.ProcessStartTime),
                            app_name: from_wide(&i.strAppName),
                            kind: app_kind(i.ApplicationType).into(),
                            service_short_name: Some(from_wide(&i.strServiceShortName)).filter(|s| !s.is_empty()),
                        })
                        .collect());
                }
                234 => infos = vec![RM_PROCESS_INFO::default(); needed as usize + 8], // ERROR_MORE_DATA
                c => return Err(OpError::from_code(c)),
            }
        }
        Err(OpError::from_code(234))
    })();
    // SAFETY: 上で始めたセッションを 1 回だけ終える
    unsafe {
        let _ = RmEndSession(session);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn self_id() -> ProcId {
        use windows::Win32::System::Threading::GetCurrentProcess;
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: 自分の擬似ハンドル。4 つの FILETIME はこの関数の中の変数
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.unwrap();
        ProcId { pid: std::process::id(), create_time: crate::util::filetime_u64(c) }
    }

    #[test]
    fn general_of_self() {
        let gate = ProcessGate::new(std::process::id(), false);
        let g = general(&gate, self_id()).unwrap();
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(g.path.as_ref().unwrap().to_lowercase().ends_with(&name));
        let cl = g.command_line.as_ref().unwrap();
        assert!(!cl.is_empty() && cl.to_lowercase().contains(exe.file_stem().unwrap().to_string_lossy().to_lowercase().as_str()));
        assert_eq!(g.user.as_ref().ok(), crate::elevation::current_user().as_ref());
        assert_eq!(g.elevated, Ok(crate::elevation::is_elevated()));
        assert!(matches!(g.integrity.as_deref(), Ok("中") | Ok("高")));
        assert_eq!(g.arch, Ok(Arch::X64));
        assert!(g.cpu_time.is_ok());
        assert_eq!(gate.open_counts(), vec![(std::process::id(), 1)]);
    }

    #[test]
    fn modules_of_self() {
        let gate = ProcessGate::new(std::process::id(), false);
        let m = modules(&gate, self_id()).unwrap();
        let sys = format!("{}\\system32\\", crate::util::system_root().to_lowercase());
        for want in ["ntdll.dll", "kernel32.dll"] {
            let row = m.iter().find(|r| r.name.eq_ignore_ascii_case(want)).unwrap_or_else(|| panic!("{want}"));
            assert!(row.path.to_lowercase().starts_with(&sys), "{want} の場所");
        }
    }

    #[test]
    fn process_and_thread_handles_are_not_duplicated() {
        assert!(never_duplicate("Process") && never_duplicate("Thread"));
        assert!(!never_duplicate("File") && !never_duplicate("Event") && !never_duplicate("Key"));
    }

    #[test]
    fn touch_free_is_refused() {
        let gate = Arc::new(ProcessGate::new(1, false));
        let id = self_id();
        gate.set_touch_free(HashSet::from([id]));
        assert_eq!(general(&gate, id).err(), Some(OpError::Gate(GateError::TouchFree)));
        assert_eq!(modules(&gate, id).err(), Some(OpError::Gate(GateError::TouchFree)));
        assert_eq!(handles(&gate, id).err(), Some(OpError::Gate(GateError::TouchFree)));
        assert!(gate.open_counts().is_empty());
    }

    #[test]
    fn handle_names_of_own_events() {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::CreateEventW;
        let tag = format!("ProcSightTest_{}", std::process::id());
        let evs: Vec<HANDLE> = (0..5)
            .map(|i| {
                let n = wide(&format!("Local\\{tag}_{i}"));
                // SAFETY: 名前付きイベントを作るだけ。最後に閉じる
                unsafe { CreateEventW(None, true, false, pcwstr(&n)) }.unwrap()
            })
            .collect();
        let gate = Arc::new(ProcessGate::new(std::process::id(), false));
        let table = handles(&gate, self_id()).unwrap();
        let start = crate::util::tick_ms();
        while !table.lock().unwrap().finished {
            assert!(crate::util::tick_ms() - start < 60_000, "名前が 60 秒で埋まらない");
            std::thread::sleep(Duration::from_millis(50));
        }
        let t = table.lock().unwrap();
        assert_eq!(t.names_done, t.names_total);
        let hits = t
            .rows
            .iter()
            .filter(|r| r.type_name == "Event" && matches!(&r.name, HandleName::Name(n) if n.contains(&tag)))
            .count();
        assert_eq!(hits, 5);
        let pt: Vec<&HandleRow> = t.rows.iter().filter(|r| r.type_name == "Process" || r.type_name == "Thread").collect();
        assert!(!pt.is_empty() && pt.iter().all(|r| r.name == HandleName::None), "Process・Thread に名前がある");
        let n = |f: fn(&HandleName) -> bool| t.rows.iter().filter(|r| f(&r.name)).count();
        println!(
            "自分のハンドル {} 件: 名前あり {}、名前なし {}、時間切れ {}、打ち切り {}、失敗 {}、{} ミリ秒",
            t.rows.len(),
            n(|x| matches!(x, HandleName::Name(_))),
            n(|x| matches!(x, HandleName::None)),
            n(|x| matches!(x, HandleName::TimedOut)),
            n(|x| matches!(x, HandleName::Skipped)),
            n(|x| matches!(x, HandleName::Failed(_))),
            crate::util::tick_ms() - start
        );
        let mut fails: Vec<String> =
            t.rows.iter().filter_map(|r| if let HandleName::Failed(c) = r.name { Some(format!("{}:{c:#x}", r.type_name)) } else { None }).collect();
        fails.sort();
        fails.dedup();
        println!("失敗の種類: {}", fails.join(", "));
        drop(t);
        for e in evs {
            // SAFETY: 上で作ったハンドルを 1 回だけ閉じる
            unsafe {
                let _ = CloseHandle(e);
            }
        }
    }

    /// 読みかけで止まっている無名パイプ(同期)の名前の問い合わせは返らない(事実22)。ワーカーを捨てて進み、
    /// 3 つ捨てた後の File は打ち切る(D-22)
    #[test]
    fn stuck_pipes_time_out_and_skip() {
        use std::io::{Read, Write};
        let mut writers = Vec::new();
        let mut readers = Vec::new();
        for _ in 0..4 {
            let (mut r, w) = std::io::pipe().unwrap();
            writers.push(w);
            readers.push(std::thread::spawn(move || {
                let mut b = [0u8; 1];
                let _ = r.read(&mut b);
            }));
        }
        std::thread::sleep(Duration::from_millis(500)); // 4 つとも読み始めて止まるのを待つ
        let gate = Arc::new(ProcessGate::new(std::process::id(), false));
        let table = handles(&gate, self_id()).unwrap();
        let start = crate::util::tick_ms();
        while !table.lock().unwrap().finished {
            assert!(crate::util::tick_ms() - start < 30_000, "30 秒で終わらない");
            std::thread::sleep(Duration::from_millis(50));
        }
        let elapsed = crate::util::tick_ms() - start;
        let t = table.lock().unwrap();
        // 忙しいときは止まらない問い合わせも 500 ミリ秒を超えうるので、数えるのは File の時間切れだけ
        let timed_out = t.rows.iter().filter(|r| r.type_name == "File" && r.name == HandleName::TimedOut).count();
        let skipped = t.rows.iter().filter(|r| r.name == HandleName::Skipped).count();
        println!("止まるパイプ: 時間切れ {timed_out}、打ち切り {skipped}、{elapsed} ミリ秒");
        assert_eq!(timed_out, MAX_ABANDONED as usize, "打ち切り {skipped}");
        assert!(skipped >= 1);
        assert!(t.rows.iter().filter(|r| r.name == HandleName::Skipped).all(|r| r.type_name == "File"));
        drop(t);
        for mut w in writers {
            let _ = w.write_all(b"x");
        }
        for r in readers {
            let _ = r.join();
        }
    }

    #[test]
    fn signatures() {
        let cache = SignatureCache::new();
        let root = crate::util::system_root();
        let mut tally = Vec::new();
        for exe in ["notepad.exe", "cmd.exe", "svchost.exe", "taskmgr.exe", "explorer.exe", "conhost.exe"] {
            let p = if exe == "explorer.exe" { format!("{root}\\{exe}") } else { format!("{root}\\System32\\{exe}") };
            let s = cache.check(&p);
            tally.push(format!("{exe}={}", s.status.label()));
            if exe == "notepad.exe" || exe == "cmd.exe" {
                assert!(matches!(s.status, SigStatus::Embedded | SigStatus::Catalog), "{exe}: {:?}", s.status);
                assert!(s.signer.as_deref().unwrap_or("").contains("Microsoft"), "{exe}");
            }
        }
        println!("署名の内訳: {}", tally.join(", "));
        let me = std::env::current_exe().unwrap();
        assert_eq!(cache.check(&me.to_string_lossy()).status, SigStatus::Unsigned);
        let gone = cache.check("C:\\no\\such\\file.exe");
        assert_eq!(gone.status, SigStatus::Failed("ファイルが見つかりません".into()));
        // 鍵が同じなら覚えた物を返す
        assert_eq!(cache.check("C:\\no\\such\\file.exe").checked_at, gone.checked_at);
    }

    #[test]
    fn locking_finds_self_and_refuses_folders() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("psight_b_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("psight.txt");
        let f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(&path).unwrap();
        let r = find_locking(&[path.to_string_lossy().into_owned()]).unwrap();
        assert!(r.iter().any(|p| p.pid == std::process::id()), "自分の PID が無い");
        drop(f);
        assert_eq!(
            find_locking(&[dir.to_string_lossy().into_owned()]).err(),
            Some(OpError::Blocked("フォルダは調べられません。中のファイルを選んでください"))
        );
        let many: Vec<String> = (0..101).map(|i| format!("C:\\x{i}.txt")).collect();
        assert_eq!(find_locking(&many).err(), Some(OpError::Blocked("一度に 100 ファイルまでです")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
