//! probe の [spi] の 1 周の時間と、[cmdline] から [net] までの節(仕様書 §9.5)。
//! 担当 B の関数は try_b で呼び、未実装なら「実装待ち」と出して次の節へ進む。

use crate::{Testee, find, ms, stats, try_b};
use procsight_win::collector::Sampler;
use procsight_win::details::{self, SignatureCache};
use procsight_win::jobs::{HandleName, SigStatus};
use procsight_win::nt::spi::{Fields, SpiReader};
use procsight_win::{ProcessGate, net, pdh, static_info};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// NFR-3: Collector の 1 周(SPI の取得・窓の一覧・diff)と、全体の値(PDH・メモリ・ネット)の時間
pub fn cycle_time() {
    let mut s = Sampler::new();
    let empty = Default::default();
    let touch = Default::default();
    let (_, gpu) = s.perf();
    let mut prev = s.rows(None, &gpu, &touch, &empty).ok();
    let (mut rows_ms, mut perf_ms) = (Vec::new(), Vec::new());
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        let (_, gpu) = s.perf();
        perf_ms.push(ms(t.elapsed()));
        let t = Instant::now();
        prev = s.rows(prev.as_ref(), &gpu, &touch, &empty).ok();
        rows_ms.push(ms(t.elapsed()));
    }
    println!("  1 周の行の作成(SPI・窓の一覧・diff。NFR-3、10 回): {}", stats(rows_ms));
    println!("  1 周の全体の値(PDH・メモリ・ネット、10 回): {}", stats(perf_ms));
}

pub fn cmdline(gate: &Arc<ProcessGate>) {
    println!("[cmdline]");
    let Some(me) = find(std::process::id()) else { return };
    let Some(info) = try_b(|| details::general(gate, me.id)) else { return };
    match info.map(|g| g.command_line) {
        Ok(Ok(c)) => {
            let ok = std::env::args().all(|a| c.contains(&a));
            println!("  情報の種類 60 で取れた: {}", if ok { "一致" } else { "不一致" });
        }
        Ok(Err(e)) | Err(e) => println!("  取れない: {}", e.message(gate.elevated())),
    }
}

/// %APPDATA%\ProcSight\config.toml の触れないプロセス(あれば)。[critical] で開かない
fn touch_free_from_config() -> procsight_core::rules::TouchFreeSet {
    let path = std::env::var("APPDATA").map(|a| std::path::Path::new(&a).join("ProcSight").join("config.toml"));
    path.ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| procsight_core::config::Config::parse(&t).ok())
        .map(|(c, _)| c.touch_free_set())
        .unwrap_or_default()
}

pub fn critical(gate: &ProcessGate) {
    println!("[critical]");
    let Ok(ps) = SpiReader::new().read(Fields::NONE) else { return };
    let touch = touch_free_from_config().resolve(&procsight_core::RawSnapshot { processes: ps.clone(), ..Default::default() });
    gate.set_touch_free(touch.clone());
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let (mut unknown, mut skipped) = (0, 0);
    for p in ps.iter().filter(|p| p.id.pid != 0 && p.id.pid != 4) {
        if touch.contains(&p.id) {
            skipped += 1;
            continue;
        }
        match static_info::fetch(gate, p.id).critical {
            Some(true) => *names.entry(p.image_name.clone()).or_default() += 1,
            Some(false) => {}
            None => unknown += 1,
        }
    }
    let list: Vec<String> = names.iter().map(|(n, c)| if *c > 1 { format!("{n} ×{c}") } else { n.clone() }).collect();
    println!("  真だった物: {}", if list.is_empty() { "なし".into() } else { list.join(", ") });
    println!("  取れなかった数: {unknown}(触れないプロセスとして開かなかった数: {skipped})");
}

pub fn pdh() {
    println!("[pdh]");
    for (path, r) in pdh::probe_counters() {
        match r {
            Ok(names) => {
                let head: Vec<&str> = names.iter().take(5).map(String::as_str).collect();
                println!("  {path}: 足せた。インスタンス {} 個。先頭: {}", names.len(), head.join(" | "));
            }
            Err(e) => println!("  {path}: 足せない(0x{e:08X})"),
        }
    }
}

pub fn cpu_compare() {
    println!("[cpu-compare]");
    let mut s = pdh::PdhSampler::open();
    let _ = s.sample();
    let f = |v: Option<f64>| v.map_or("—".to_string(), |v| format!("{v:6.1}"));
    for i in 1..=30 {
        std::thread::sleep(Duration::from_secs(1));
        let v = s.sample();
        println!("  {i:2} 秒  Utility {}  Time {}", f(v.cpu_utility_total), f(v.cpu_time_total));
        if i % 10 == 0 {
            println!("  ── 今のタスク マネージャーの CPU の値を書いてください({i} 秒目)");
        }
    }
}

pub fn handles(gate: &Arc<ProcessGate>) {
    println!("[handles]");
    let t = Testee::start(&["--handles", "1000", "--pipe"]);
    std::thread::sleep(Duration::from_secs(1));
    let start = Instant::now();
    let Some(r) = try_b(|| details::handles(gate, t.id)) else { return };
    let table = match r {
        Ok(t) => t,
        Err(e) => {
            println!("  取れない: {}", e.message(gate.elevated()));
            return;
        }
    };
    println!("  一覧の取得: {:.1} ミリ秒", ms(start.elapsed()));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let done = table.lock().map(|t| t.finished).unwrap_or(true);
        if done || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let tab = table.lock().unwrap_or_else(|e| e.into_inner());
    println!("  testee のハンドル: {} 件、名前を埋め終えた: {}(名前 {} / {})", tab.rows.len(), tab.finished, tab.names_done, tab.names_total);
    let events = tab
        .rows
        .iter()
        .filter(|r| r.type_name == "Event" && matches!(&r.name, HandleName::Name(n) if n.contains("ProcSightTest_")))
        .count();
    println!("  Event の名前に ProcSightTest_ を含む行: {events}(3 つ以上取れたか: {})", if events >= 3 { "はい" } else { "いいえ" });
    let pipe = format!("ProcSightTest_{}", t.id.pid);
    let pipe_rows: Vec<&HandleName> = tab
        .rows
        .iter()
        .filter(|r| r.type_name == "File")
        .map(|r| &r.name)
        .filter(|n| matches!(n, HandleName::Name(s) if s.contains(&pipe)) || matches!(n, HandleName::TimedOut))
        .collect();
    let returned = pipe_rows.iter().any(|n| matches!(n, HandleName::Name(_)));
    let timed_out = pipe_rows.iter().filter(|n| matches!(n, HandleName::TimedOut)).count();
    println!("  パイプのハンドルの名前が 500 ミリ秒以内に返った: {}(応答なしの File: {timed_out})", if returned { "はい" } else { "いいえ" });
}

pub fn signature() {
    println!("[signature]");
    let dir = std::path::Path::new(&procsight_win::util::system_root()).join("System32");
    let mut exes: Vec<String> = std::fs::read_dir(&dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    exes.sort();
    exes.truncate(20);
    let Some(cache) = try_b(SignatureCache::new) else { return };
    let start = Instant::now();
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &exes {
        let Some(s) = try_b(|| cache.check(e)) else { return };
        let k = match s.status {
            SigStatus::Embedded => "埋め込み",
            SigStatus::Catalog => "カタログ",
            SigStatus::Unsigned => "署名なし",
            SigStatus::Failed(_) => "失敗",
        };
        *tally.entry(k).or_default() += 1;
    }
    println!("  {} 個: {:?}、{:.0} ミリ秒", exes.len(), tally, ms(start.elapsed()));
}

pub fn net() {
    println!("[net]");
    match net::counts() {
        Some((all, up, shown)) => println!("  アダプター {all} 個、Up {up} 個(ループバックとフィルターを除いて出す物 {shown} 個)"),
        None => println!("  GetIfTable2 が失敗しました"),
    }
}
