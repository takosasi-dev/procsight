//! Phase 0 の確かめ用 CLI(仕様書 §9.5)。非公開の API と位置を公開 API と突き合わせ、PDH・ネット・署名などの実物を標準出力に出す。
//! 名前・パス・ユーザー名は出さない(testee と ProcSight 自身の名前、[critical] の名前だけは出す)。
//! `ifeo` は管理者で利用者が手で動かす副命令。`ifeo-echo` は置き換えから起動されたときの記録だけをする。

use procsight_core::diff::{THREAD_STATE_WAITING, WAIT_REASON_SUSPENDED};
use procsight_core::{ProcId, RawProcess};
use procsight_win::ProcessGate;
use procsight_win::nt::spi::{self, Fields, SpiReader};
use procsight_win::static_info;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

mod ifeo;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ifeo") => ifeo::run(),
        Some("ifeo-echo") => ifeo::echo(&args[1..]),
        Some(other) => {
            eprintln!("知らない引数: {other}(引数なし・ifeo・ifeo-echo)");
            std::process::exit(2);
        }
        None => run_all(),
    }
}

/// 自分が起動した testee。Drop で必ず終わらせる(一時停止中でも TerminateProcess で終わる)
pub struct Testee {
    child: Child,
    pub id: ProcId,
}

impl Testee {
    pub fn start(args: &[&str]) -> Testee {
        let exe = std::env::current_exe().expect("probe の exe").with_file_name("procsight-testee.exe");
        let child = Command::new(&exe)
            .args(args)
            .args(["--exit-after", "180"])
            .spawn()
            .unwrap_or_else(|e| panic!("procsight-testee.exe を probe と同じフォルダに置いてください({e})"));
        // 先に Testee にして、見つからずに panic しても Drop で終わらせる
        let mut t = Testee { id: ProcId { pid: child.id(), create_time: 0 }, child };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(p) = find(t.id.pid) {
                t.id = p.id;
                return t;
            }
            assert!(Instant::now() < deadline, "testee が SPI に現れない");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Testee {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn find(pid: u32) -> Option<RawProcess> {
    SpiReader::new().read(Fields::ALL).ok()?.into_iter().find(|p| p.id.pid == pid)
}

fn cpu_time(pid: u32) -> Option<u64> {
    let p = find(pid)?;
    Some(p.kernel_time? + p.user_time?)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// B の関数(担当 B)が未実装(todo!)の間は panic するので、その節は「担当 B 待ち」と出して続ける
fn try_b<T>(f: impl FnOnce() -> T) -> Option<T> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f)).ok();
    std::panic::set_hook(prev);
    if r.is_none() {
        println!("  担当 B の実装待ち(呼べませんでした)");
    }
    r
}

fn run_all() {
    let me = std::process::id();
    let elevated = static_info::self_elevated();
    let gate = std::sync::Arc::new(ProcessGate::new(me, elevated));
    env(elevated);
    spi_section();
    spi_verify(&gate);
    threads_and_suspend(&gate);
    sections::cmdline(&gate);
    sections::critical(&gate);
    sections::pdh();
    sections::cpu_compare();
    sections::handles(&gate);
    sections::signature();
    sections::net();
}

mod sections;

fn env(elevated: bool) {
    println!("[env]");
    match static_info::windows_version() {
        Some((a, b, c)) => println!("  Windows {a}.{b}.{c}"),
        None => println!("  Windows の版: 取れません"),
    }
    println!("  論理プロセッサ: {}", procsight_win::util::logical_cpus());
    println!("  プロセッサのグループ: {}", procsight_win::util::processor_groups());
    println!("  昇格: {}", if elevated { "はい" } else { "いいえ" });
    println!("  NtSuspendProcess / NtResumeProcess: {}", if procsight_win::nt::suspend::available() { "取れた" } else { "取れない" });
}

fn stats(mut v: Vec<f64>) -> String {
    v.sort_by(f64::total_cmp);
    format!("最小 {:.2} / 中央 {:.2} / 最大 {:.2} ミリ秒", v[0], v[v.len() / 2], v[v.len() - 1])
}

fn spi_section() {
    println!("[spi]");
    let mut reader = SpiReader::new();
    let mut times = Vec::new();
    let (mut procs, mut threads) = (0, 0);
    for _ in 0..10 {
        let t = Instant::now();
        match reader.read(Fields::ALL) {
            Ok(ps) => {
                times.push(ms(t.elapsed()));
                procs = ps.len();
                threads = ps.iter().map(|p| p.threads.len()).sum();
            }
            Err(s) => println!("  失敗: NTSTATUS 0x{:08X}", s as u32),
        }
    }
    if times.is_empty() {
        return;
    }
    println!("  1 回の取得(読み取りと解釈、10 回): {}", stats(times));
    println!("  プロセス {procs} / スレッド {threads}");
    println!("  バッファ: 返った大きさ {} KB / 確保 {} KB", reader.last_len() / 1024, reader.capacity() / 1024);
    sections::cycle_time();
}

fn print_checks(checks: &[spi::FieldCheck]) {
    for c in checks {
        let v = |o: Option<u64>| o.map_or("—".to_string(), |v| v.to_string());
        let d = c.diff().map_or("—".to_string(), |d| d.to_string());
        println!(
            "  {:<20} SPI {:>20}  公開 API(前 {} / 後 {})  {}  差 {}",
            c.field,
            v(c.spi),
            v(c.before),
            v(c.after),
            c.verdict.label(),
            d
        );
    }
}

fn spi_verify(gate: &ProcessGate) {
    println!("[spi-verify]");
    println!("  自分(procsight-probe)");
    let s = spi::self_check();
    print_checks(&s.checks);
    println!("  → 使う項目: {:?}(CreateTime の一致: {})", s.fields, s.create_time_ok);
    println!("  testee(procsight-testee --busy 1 --write-rate 20、5 秒後)");
    let t = Testee::start(&["--busy", "1", "--write-rate", "20"]);
    std::thread::sleep(Duration::from_secs(5));
    match gate.open(t.id, Default::default()) {
        Ok(h) => match spi::verify_handle(h.raw(), t.id.pid) {
            Ok(c) => print_checks(&c),
            Err(s) => println!("  SPI の失敗: 0x{:08X}", s as u32),
        },
        Err(e) => println!("  testee を開けません: {e:?}"),
    }
}

fn thread_line(p: &RawProcess) -> String {
    let mut pairs: std::collections::BTreeMap<(u32, u32), usize> = Default::default();
    for t in &p.threads {
        *pairs.entry((t.state, t.wait_reason)).or_default() += 1;
    }
    pairs.iter().map(|((s, w), n)| format!("State={s} WaitReason={w} ×{n}")).collect::<Vec<_>>().join(", ")
}

fn threads_and_suspend(gate: &ProcessGate) {
    println!("[threads]");
    let t = Testee::start(&["--busy", "1", "--threads", "3"]);
    std::thread::sleep(Duration::from_millis(500));
    if let Some(p) = find(t.id.pid) {
        println!("  止める前: スレッド {} 本: {}", p.threads.len(), thread_line(&p));
    }
    let suspended = procsight_win::ops::suspend(gate, t.id);
    std::thread::sleep(Duration::from_millis(300));
    match &suspended {
        Ok(()) => {
            if let Some(p) = find(t.id.pid) {
                println!("  止めた後: スレッド {} 本: {}", p.threads.len(), thread_line(&p));
                let all = p.threads.iter().all(|t| t.state == THREAD_STATE_WAITING && t.wait_reason == WAIT_REASON_SUSPENDED);
                println!(
                    "  全スレッドが State={THREAD_STATE_WAITING} WaitReason={WAIT_REASON_SUSPENDED}(core の値): {}",
                    if all { "はい" } else { "いいえ" }
                );
            }
        }
        Err(e) => println!("  NtSuspendProcess の失敗: {e:?}"),
    }
    println!("[suspend]");
    println!("  NtSuspendProcess / NtResumeProcess: {}", if procsight_win::nt::suspend::available() { "取れた" } else { "取れない" });
    if suspended.is_err() {
        return;
    }
    let c0 = cpu_time(t.id.pid);
    std::thread::sleep(Duration::from_secs(5));
    let c1 = cpu_time(t.id.pid);
    let r = procsight_win::ops::resume(gate, t.id);
    let c2 = cpu_time(t.id.pid);
    std::thread::sleep(Duration::from_secs(5));
    let c3 = cpu_time(t.id.pid);
    let inc = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => format!("{:.1} ミリ秒", b.saturating_sub(a) as f64 / 10_000.0),
        _ => "—".into(),
    };
    println!("  止めた後 5 秒の CPU 時間の増分: {}", inc(c0, c1));
    println!("  再開: {}", if r.is_ok() { "成功".to_string() } else { format!("{r:?}") });
    println!("  再開した後 5 秒の CPU 時間の増分: {}", inc(c2, c3));
}
