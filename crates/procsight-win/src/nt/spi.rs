//! NtQuerySystemInformation(SystemProcessInformation) の読み取り(担当 A。D-4・事実4・事実19)。
//! 構造体は windows crate の型を使わず、バイトの位置で読む。Reserved の中の位置は probe で公開 API と突き合わせた物だけ使う。
//! 起動時の自己確認(INTERFACES §3.2)もここ。合わなかった項目は読まずに None にする(D-6)。

use crate::nt::STATUS_INFO_LENGTH_MISMATCH;
use crate::util::filetime_u64;
use procsight_core::{ProcId, RawProcess, RawThread};
use std::sync::atomic::{AtomicU8, Ordering};
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, GetProcessIoCounters, GetProcessTimes, GetThreadTimes,
    IO_COUNTERS,
};

/// バッファの上限(E-9)
pub const MAX_BUFFER: usize = 64 * 1024 * 1024;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
const ENTRY_MIN: usize = 0x100;
const THREAD_SIZE: usize = 0x50;

/// Reserved の中の項目のうち、読んでよい物(自己確認で公開 API と合った物)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Fields {
    /// プロセスの KernelTime・UserTime
    pub cpu_times: bool,
    /// スレッドの KernelTime・UserTime・CreateTime
    pub thread_times: bool,
    pub private_ws: bool,
    pub io: bool,
}

impl Fields {
    pub const ALL: Fields = Fields { cpu_times: true, thread_times: true, private_ws: true, io: true };
    pub const NONE: Fields = Fields { cpu_times: false, thread_times: false, private_ws: false, io: false };
    fn bits(self) -> u8 {
        self.cpu_times as u8 | (self.thread_times as u8) << 1 | (self.private_ws as u8) << 2 | (self.io as u8) << 3
    }
    fn from_bits(b: u8) -> Fields {
        Fields { cpu_times: b & 1 != 0, thread_times: b & 2 != 0, private_ws: b & 4 != 0, io: b & 8 != 0 }
    }
}

/// 自己確認の前は何も信じない
static FIELDS: AtomicU8 = AtomicU8::new(0);

/// 自己確認の結果を入れる(Backend の起動時)
pub fn set_fields(f: Fields) {
    FIELDS.store(f.bits(), Ordering::Relaxed);
}

pub fn fields() -> Fields {
    Fields::from_bits(FIELDS.load(Ordering::Relaxed))
}

/// SPI のバッファを持ち回る読み手。Collector は 1 つを使い続ける(毎回の確保を避ける)
pub struct SpiReader {
    buf: Vec<u64>,
    len: usize,
}

impl Default for SpiReader {
    fn default() -> Self {
        Self::new()
    }
}

impl SpiReader {
    pub fn new() -> SpiReader {
        SpiReader { buf: vec![0u64; 512 * 1024 / 8], len: 0 }
    }

    /// 今のバッファの大きさ(バイト)
    pub fn capacity(&self) -> usize {
        self.buf.len() * 8
    }

    /// 前回返ったデータの大きさ(バイト)
    pub fn last_len(&self) -> usize {
        self.len
    }

    /// E-9: 足りなければ、返された大きさに 64 KB を足して取り直す。4 回で取れないか 64 MB を超えたら Err(NTSTATUS)
    fn fill(&mut self) -> Result<&[u8], i32> {
        let mut last = STATUS_INFO_LENGTH_MISMATCH;
        for _ in 0..4 {
            let mut ret = 0u32;
            let cap = self.capacity();
            // SAFETY: buf は cap バイトの書ける領域(u64 の並びなので 8 バイトに揃っている)。ret はこの関数の変数
            let status = unsafe {
                NtQuerySystemInformation(SystemProcessInformation, self.buf.as_mut_ptr().cast(), cap as u32, &mut ret)
            };
            if status.0 >= 0 {
                self.len = (ret as usize).min(cap);
                // SAFETY: buf の先頭から len バイト(len <= cap)を u8 として読むだけ。buf は self が持ち、戻り値の寿命は &mut self に縛られる
                return Ok(unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.len) });
            }
            last = status.0;
            if status.0 != STATUS_INFO_LENGTH_MISMATCH && status.0 != STATUS_BUFFER_TOO_SMALL {
                return Err(status.0);
            }
            let want = if ret as usize > cap { ret as usize } else { cap * 2 } + 64 * 1024;
            if want > MAX_BUFFER {
                return Err(status.0);
            }
            self.buf = vec![0u64; want.div_ceil(8)];
        }
        Err(last)
    }

    pub fn read(&mut self, fields: Fields) -> Result<Vec<RawProcess>, i32> {
        let bytes = self.fill()?;
        Ok(parse(bytes, fields))
    }
}

fn rd<const N: usize>(b: &[u8], off: usize) -> Option<[u8; N]> {
    b.get(off..off.checked_add(N)?)?.try_into().ok()
}
fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    rd::<2>(b, off).map(u16::from_le_bytes)
}
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    rd::<4>(b, off).map(u32::from_le_bytes)
}
fn i32_at(b: &[u8], off: usize) -> Option<i32> {
    rd::<4>(b, off).map(i32::from_le_bytes)
}
fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    rd::<8>(b, off).map(u64::from_le_bytes)
}

/// ImageName の Buffer はバッファの中を指すポインタ。位置に直して範囲を確かめてから読む(生のポインタを辿らない)
fn image_name(b: &[u8], entry: usize) -> String {
    let (Some(len), Some(ptr)) = (u16_at(b, entry + 0x38), u64_at(b, entry + 0x40)) else {
        return String::new();
    };
    let base = b.as_ptr() as u64;
    if ptr < base || len == 0 {
        return String::new();
    }
    let start = (ptr - base) as usize;
    let Some(bytes) = start.checked_add(len as usize).and_then(|end| b.get(start..end)) else {
        return String::new();
    };
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// SPI のバッファを読む。範囲の外の位置は読まない(壊れた・短いバッファでも panic しない)
pub fn parse(b: &[u8], f: Fields) -> Vec<RawProcess> {
    let mut out = Vec::with_capacity(512);
    let mut off = 0usize;
    loop {
        if off.checked_add(ENTRY_MIN).is_none_or(|end| end > b.len()) {
            break;
        }
        let next = u32_at(b, off).unwrap_or(0) as usize;
        let end = if next == 0 { b.len() } else { (off + next).min(b.len()) };
        let n_threads = u32_at(b, off + 4).unwrap_or(0) as usize;
        let at = |o: usize| u64_at(b, off + o);
        let mut threads = Vec::with_capacity(n_threads.min(4096));
        for i in 0..n_threads {
            let t = off + ENTRY_MIN + i * THREAD_SIZE;
            if t + THREAD_SIZE > end {
                break;
            }
            let tu = |o: usize| u64_at(b, t + o);
            threads.push(RawThread {
                tid: tu(0x30).unwrap_or(0) as u32,
                start_address: tu(0x20).unwrap_or(0),
                priority: i32_at(b, t + 0x38).unwrap_or(0),
                base_priority: i32_at(b, t + 0x3C).unwrap_or(0),
                state: u32_at(b, t + 0x44).unwrap_or(0),
                wait_reason: u32_at(b, t + 0x48).unwrap_or(0),
                kernel_time: if f.thread_times { tu(0x00) } else { None },
                user_time: if f.thread_times { tu(0x08) } else { None },
                create_time: if f.thread_times { tu(0x10) } else { None },
            });
        }
        out.push(RawProcess {
            id: ProcId { pid: at(0x50).unwrap_or(0) as u32, create_time: at(0x20).unwrap_or(0) },
            parent_pid: at(0x58).unwrap_or(0) as u32,
            image_name: image_name(b, off),
            session_id: u32_at(b, off + 0x64).unwrap_or(0),
            base_priority: i32_at(b, off + 0x48).unwrap_or(0),
            handle_count: u32_at(b, off + 0x60).unwrap_or(0),
            threads,
            kernel_time: if f.cpu_times { at(0x30) } else { None },
            user_time: if f.cpu_times { at(0x28) } else { None },
            working_set: at(0x90).unwrap_or(0),
            private_working_set: if f.private_ws { at(0x08) } else { None },
            private_bytes: at(0xC8).unwrap_or(0),
            io_read_bytes: if f.io { at(0xE8) } else { None },
            io_write_bytes: if f.io { at(0xF0) } else { None },
        });
        if next == 0 || next < ENTRY_MIN {
            break;
        }
        off += next;
    }
    out
}

/// 名前(大文字・小文字を区別しない)が一致する動いているプロセスの識別子。D-40 で Taskmgr.exe を探すのに使う
pub fn pids_named(name: &str) -> Vec<ProcId> {
    let want = name.to_lowercase();
    SpiReader::new()
        .read(Fields::NONE)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.threads.is_empty() && p.image_name.to_lowercase() == want)
        .map(|p| p.id)
        .collect()
}

/// 1 回分の全プロセス。失敗したら NTSTATUS
pub fn query_processes() -> Result<Vec<RawProcess>, i32> {
    SpiReader::new().read(fields())
}

// ---- 公開 API との突き合わせ(起動時の自己確認と probe の [spi-verify]) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Match,
    Mismatch,
    /// 公開 API が取れない、または値が 0 で違いを見分けられない
    NotComparable,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Match => "一致",
            Verdict::Mismatch => "不一致",
            Verdict::NotComparable => "比べられない",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldCheck {
    pub field: &'static str,
    pub spi: Option<u64>,
    /// SPI を読む直前と直後の公開 API の値
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub verdict: Verdict,
}

impl FieldCheck {
    /// SPI の値と、公開 API の 2 回の値のうち近い方との差(符号つき)
    pub fn diff(&self) -> Option<i128> {
        let s = self.spi? as i128;
        [self.before, self.after].into_iter().flatten().map(|v| s - v as i128).min_by_key(|d| d.abs())
    }
}

#[derive(Default, Clone, Copy)]
struct ApiValues {
    create: Option<u64>,
    kernel: Option<u64>,
    user: Option<u64>,
    private_ws: Option<u64>,
    read: Option<u64>,
    write: Option<u64>,
}

fn api_values(h: HANDLE) -> ApiValues {
    let mut v = ApiValues::default();
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: h は呼ぶ側が開いた(または擬似の)有効なハンドル。4 つの FILETIME はこの関数の変数
    if unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.is_ok() {
        v.create = Some(filetime_u64(c));
        v.kernel = Some(filetime_u64(k));
        v.user = Some(filetime_u64(u));
    }
    let mut m = PROCESS_MEMORY_COUNTERS_EX2 { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32, ..Default::default() };
    // SAFETY: m は EX2 の大きさの構造体で、cb にその大きさを入れて渡す(関数は先頭を PROCESS_MEMORY_COUNTERS として受ける)
    let ok = unsafe { GetProcessMemoryInfo(h, (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(), m.cb) };
    // 0 は「EX2 に対応していない Windows が埋めなかった」と見分けられないので、比べられない扱い
    if ok.is_ok() && m.PrivateWorkingSetSize > 0 {
        v.private_ws = Some(m.PrivateWorkingSetSize as u64);
    }
    let mut io = IO_COUNTERS::default();
    // SAFETY: io はこの関数の変数
    if unsafe { GetProcessIoCounters(h, &mut io) }.is_ok() {
        v.read = Some(io.ReadTransferCount);
        v.write = Some(io.WriteTransferCount);
    }
    v
}

/// タイマーの 1 刻み(既定の 15.625 ミリ秒、100 ナノ秒単位)
const TICK: u64 = 156_250;

/// 単調に増える値の判定。SPI を読む前(before)と後(after)の公開 API で挟み、before − slack <= SPI <= after + slack なら一致。
/// 公開 API が 0 のままなら、位置が違っても 0 を読んで合ってしまうので比べられない扱い
fn bracket(field: &'static str, spi: Option<u64>, before: Option<u64>, after: Option<u64>, slack: u64) -> FieldCheck {
    let verdict = match (spi, before, after) {
        (Some(s), Some(a), Some(b)) if a == 0 && b == 0 && s == 0 => Verdict::NotComparable,
        (Some(s), Some(a), Some(b)) if a.saturating_sub(slack) <= s && s <= b.saturating_add(slack) => Verdict::Match,
        (Some(_), Some(_), Some(_)) => Verdict::Mismatch,
        _ => Verdict::NotComparable,
    };
    FieldCheck { field, spi, before, after, verdict }
}

/// 増えも減りもする値(メモリ)の判定。2 回の公開 API の値の範囲を、幅 tol だけ広げた中に入れば一致
fn band(field: &'static str, spi: Option<u64>, before: Option<u64>, after: Option<u64>) -> FieldCheck {
    let verdict = match (spi, before, after) {
        (Some(s), Some(a), Some(b)) => {
            let (lo, hi) = (a.min(b), a.max(b));
            // 読む間にも他のスレッドがページに触れて数百 KB は動く。位置を間違えたときの値(数・ポインタ・別の量)は
            // 桁が違うので、1 MiB か 2% の広い方を許しても見分けられる
            let tol = (1u64 << 20).max(hi / 50);
            if lo.saturating_sub(tol) <= s && s <= hi + tol { Verdict::Match } else { Verdict::Mismatch }
        }
        _ => Verdict::NotComparable,
    };
    FieldCheck { field, spi, before, after, verdict }
}

/// 自己確認の結果
#[derive(Clone, Debug)]
pub struct SelfCheck {
    pub checks: Vec<FieldCheck>,
    pub fields: Fields,
    /// CreateTime の位置が合ったか。合わなければ識別子が作れず、関門で開く操作は全部「入れ替わりました」になる
    pub create_time_ok: bool,
}

fn find(reader: &mut SpiReader, pid: u32) -> Result<Option<RawProcess>, i32> {
    Ok(reader.read(Fields::ALL)?.into_iter().find(|p| p.id.pid == pid))
}

/// h(開いたハンドル)と pid のプロセスについて、候補の位置の値を公開 API と比べる。
/// 順は「公開 API → SPI → 公開 API」。バッファは比べる前に確保しておく(読む間の時間を縮める)
pub fn verify_handle(h: HANDLE, pid: u32) -> Result<Vec<FieldCheck>, i32> {
    let mut reader = SpiReader::new();
    reader.fill()?;
    let a = api_values(h);
    let p = find(&mut reader, pid)?;
    let b = api_values(h);
    let Some(p) = p else { return Err(STATUS_INFO_LENGTH_MISMATCH) };
    let create = match (Some(p.id.create_time), a.create) {
        (Some(s), Some(c)) if c != 0 => {
            FieldCheck { field: "CreateTime", spi: Some(s), before: a.create, after: b.create, verdict: if s == c { Verdict::Match } else { Verdict::Mismatch } }
        }
        _ => FieldCheck { field: "CreateTime", spi: Some(p.id.create_time), before: a.create, after: b.create, verdict: Verdict::NotComparable },
    };
    Ok(vec![
        create,
        // CPU 時間はクロックの割り込みごとに 1 刻みずつ数えられる。2 つの API が今の刻みを数えに入れるかで 1 刻みずれうるので、1 刻み許す
        bracket("KernelTime", p.kernel_time, a.kernel, b.kernel, TICK),
        bracket("UserTime", p.user_time, a.user, b.user, TICK),
        band("PrivateWorkingSet", p.private_working_set, a.private_ws, b.private_ws),
        // I/O の量はバイトの数で、刻みが無い。挟んだ範囲にそのまま入るはず
        bracket("ReadTransferCount", p.io_read_bytes, a.read, b.read, 0),
        bracket("WriteTransferCount", p.io_write_bytes, a.write, b.write, 0),
    ])
}

fn spin(ms: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut x = 1u64;
    while std::time::Instant::now() < until {
        x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
    }
}

/// 一時ファイルに書いて読み、I/O の量を 0 でなくする(0 と 0 の一致を避ける)
fn touch_io() {
    use std::io::{Read, Seek, Write};
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    let path = std::env::temp_dir().join(format!("procsight-selfcheck-{}.tmp", std::process::id()));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
    {
        let data = [0xA5u8; 64 * 1024];
        let _ = f.write_all(&data);
        let _ = f.rewind();
        let mut back = vec![0u8; data.len()];
        let _ = f.read_exact(&mut back);
    }
}

/// 自分のスレッドの時刻を GetThreadTimes(擬似ハンドル)と比べる
fn verify_thread() -> Result<Vec<FieldCheck>, i32> {
    let thread_times = || {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: GetCurrentThread は閉じる必要のない擬似ハンドル。4 つの FILETIME はこの関数の変数
        unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) }
            .ok()
            .map(|_| (filetime_u64(c), filetime_u64(k), filetime_u64(u)))
    };
    // SAFETY: 引数なし
    let tid = unsafe { GetCurrentThreadId() };
    let mut reader = SpiReader::new();
    reader.fill()?;
    let a = thread_times();
    let p = find(&mut reader, std::process::id())?;
    let b = thread_times();
    let t = p.and_then(|p| p.threads.into_iter().find(|t| t.tid == tid));
    let Some(t) = t else { return Err(STATUS_INFO_LENGTH_MISMATCH) };
    let create = FieldCheck {
        field: "Thread.CreateTime",
        spi: t.create_time,
        before: a.map(|v| v.0),
        after: b.map(|v| v.0),
        verdict: match (t.create_time, a) {
            (Some(s), Some((c, _, _))) if c != 0 => if s == c { Verdict::Match } else { Verdict::Mismatch },
            _ => Verdict::NotComparable,
        },
    };
    Ok(vec![
        create,
        bracket("Thread.KernelTime", t.kernel_time, a.map(|v| v.1), b.map(|v| v.1), TICK),
        bracket("Thread.UserTime", t.user_time, a.map(|v| v.2), b.map(|v| v.2), TICK),
    ])
}

fn ok(checks: &[FieldCheck], name: &str) -> Verdict {
    checks.iter().find(|c| c.field == name).map_or(Verdict::NotComparable, |c| c.verdict)
}

/// 起動時の自己確認。自分(GetCurrentProcess の擬似ハンドル。関門を通さない)の値で確かめ、合った項目だけを使う
pub fn self_check() -> SelfCheck {
    spin(40); // UserTime を 0 でなくする
    touch_io();
    // SAFETY: GetCurrentProcess は閉じる必要のない擬似ハンドル
    let me = unsafe { GetCurrentProcess() };
    // 読む間に他のスレッドがメモリを確保・解放すると 1 回だけ外れうる。位置が違えば毎回外れるので、不一致なら 2 回まで読み直す
    let mut checks = Vec::new();
    for _ in 0..3 {
        checks = verify_handle(me, std::process::id()).unwrap_or_default();
        if checks.iter().all(|c| c.verdict != Verdict::Mismatch) {
            break;
        }
    }
    checks.extend(verify_thread().unwrap_or_default());
    let create_time_ok = ok(&checks, "CreateTime") == Verdict::Match;
    // KernelTime は起動直後に 0 のままのことがある。CreateTime と UserTime が合えば同じ並びの KernelTime の位置も確か、とみなす(不一致なら使わない)
    let cpu_times = create_time_ok && ok(&checks, "UserTime") == Verdict::Match && ok(&checks, "KernelTime") != Verdict::Mismatch;
    let thread_times = ok(&checks, "Thread.CreateTime") == Verdict::Match
        && ok(&checks, "Thread.UserTime") != Verdict::Mismatch
        && ok(&checks, "Thread.KernelTime") != Verdict::Mismatch;
    let io = ok(&checks, "ReadTransferCount") == Verdict::Match && ok(&checks, "WriteTransferCount") == Verdict::Match;
    let fields = Fields { cpu_times, thread_times, private_ws: ok(&checks, "PrivateWorkingSet") == Verdict::Match, io };
    SelfCheck { checks, fields, create_time_ok }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 作り物のバッファ: プロセス 2 つ(2 つ目は名前つき・スレッド 1 本)
    fn fake() -> Vec<u8> {
        let first = ENTRY_MIN;
        let second = ENTRY_MIN + THREAD_SIZE;
        let mut b = vec![0u8; first + second + 32];
        b[0..4].copy_from_slice(&(first as u32).to_le_bytes());
        let o = first;
        b[o + 4..o + 8].copy_from_slice(&1u32.to_le_bytes());
        b[o + 0x08..o + 0x10].copy_from_slice(&4096u64.to_le_bytes());
        b[o + 0x20..o + 0x28].copy_from_slice(&777u64.to_le_bytes());
        b[o + 0x28..o + 0x30].copy_from_slice(&20u64.to_le_bytes());
        b[o + 0x30..o + 0x38].copy_from_slice(&10u64.to_le_bytes());
        b[o + 0x48..o + 0x4C].copy_from_slice(&8i32.to_le_bytes());
        b[o + 0x50..o + 0x58].copy_from_slice(&1234u64.to_le_bytes());
        b[o + 0x58..o + 0x60].copy_from_slice(&4u64.to_le_bytes());
        b[o + 0x60..o + 0x64].copy_from_slice(&55u32.to_le_bytes());
        b[o + 0xE8..o + 0xF0].copy_from_slice(&99u64.to_le_bytes());
        let t = o + ENTRY_MIN;
        b[t + 0x30..t + 0x38].copy_from_slice(&5678u64.to_le_bytes());
        b[t + 0x44..t + 0x48].copy_from_slice(&5u32.to_le_bytes());
        b[t + 0x48..t + 0x4C].copy_from_slice(&5u32.to_le_bytes());
        // 名前は末尾の 32 バイトに置く(UTF-16 で "ab")
        let name_off = first + second;
        b[name_off..name_off + 4].copy_from_slice(&[b'a', 0, b'b', 0]);
        b[o + 0x38..o + 0x3A].copy_from_slice(&4u16.to_le_bytes());
        let ptr = b.as_ptr() as u64 + name_off as u64;
        b[o + 0x40..o + 0x48].copy_from_slice(&ptr.to_le_bytes());
        b
    }

    #[test]
    fn parse_fake_buffer() {
        let b = fake();
        let ps = parse(&b, Fields::ALL);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].image_name, "");
        let p = &ps[1];
        assert_eq!(p.id, ProcId { pid: 1234, create_time: 777 });
        assert_eq!(p.image_name, "ab");
        assert_eq!((p.parent_pid, p.handle_count, p.base_priority), (4, 55, 8));
        assert_eq!((p.kernel_time, p.user_time, p.private_working_set, p.io_read_bytes), (Some(10), Some(20), Some(4096), Some(99)));
        assert_eq!(p.threads.len(), 1);
        assert_eq!((p.threads[0].tid, p.threads[0].state, p.threads[0].wait_reason), (5678, 5, 5));
        let hidden = parse(&b, Fields::NONE);
        assert_eq!((hidden[1].kernel_time, hidden[1].private_working_set, hidden[1].io_write_bytes), (None, None, None));
        assert_eq!(hidden[1].threads[0].create_time, None);
    }

    #[test]
    fn parse_truncated_buffer_does_not_panic() {
        let b = fake();
        for cut in [0, 10, ENTRY_MIN - 1, ENTRY_MIN + 3, b.len() - 40] {
            let _ = parse(&b[..cut], Fields::ALL);
        }
        // 名前のポインタがバッファの外
        let mut bad = fake();
        let o = ENTRY_MIN;
        bad[o + 0x40..o + 0x48].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(parse(&bad, Fields::ALL)[1].image_name, "");
    }

    #[test]
    fn real_query_finds_self() {
        let me = std::process::id();
        let ps = SpiReader::new().read(Fields::NONE).expect("SPI");
        let p = ps.iter().find(|p| p.id.pid == me).expect("自分がいる");
        assert!(!p.threads.is_empty());
        assert!(ps.iter().any(|p| p.id.pid == 4));
        assert!(!pids_named(&p.image_name.to_uppercase()).is_empty());
    }

    #[test]
    fn self_check_matches_on_this_windows() {
        let c = self_check();
        assert!(c.create_time_ok, "{:?}", c.checks);
        assert!(c.fields.cpu_times, "{:?}", c.checks);
    }

    #[test]
    fn verdicts() {
        assert_eq!(bracket("x", Some(0), Some(0), Some(0), 0).verdict, Verdict::NotComparable);
        assert_eq!(bracket("x", Some(5), Some(4), Some(6), 0).verdict, Verdict::Match);
        assert_eq!(bracket("x", Some(9), Some(4), Some(6), 0).verdict, Verdict::Mismatch);
        assert_eq!(band("m", Some(10 << 20), Some(10 << 20), Some(11 << 20)).verdict, Verdict::Match);
        assert_eq!(band("m", Some(100 << 20), Some(10 << 20), Some(11 << 20)).verdict, Verdict::Mismatch);
    }
}
