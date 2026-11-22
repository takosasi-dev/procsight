//! PDH の取得(担当 A)。CPU・物理ディスク・GPU のカウンターを英語の名前で 1 回だけ足す(D-9・D-12・D-13)。
//! ワイルドカードのカウンターを 1 本ずつ足し、PdhGetFormattedCounterArrayW で全インスタンスを読む。
//! 足せないカウンターはその値を None にして続ける(E-11)。PDH_HQUERY は作ったスレッドで使う。

use crate::util::{pwstr_to_string, wide};
use procsight_core::{DiskSample, GpuByPid, GpuSample};
use std::collections::{BTreeMap, HashMap};
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
use windows::core::PCWSTR;

/// windows crate に無い PDH_FMT_NOCAP100(% Processor Utility は 100 を超えうる。超えた分は自分で 100 に収める)
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
const FMT: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);

/// インスタンスの (名前, 値)
type Items = Vec<(String, f64)>;

pub const CPU_UTILITY: &str = "\\Processor Information(*)\\% Processor Utility";
pub const CPU_TIME: &str = "\\Processor Information(*)\\% Processor Time";
pub const DISK_IDLE: &str = "\\PhysicalDisk(*)\\% Idle Time";
pub const DISK_READ: &str = "\\PhysicalDisk(*)\\Disk Read Bytes/sec";
pub const DISK_WRITE: &str = "\\PhysicalDisk(*)\\Disk Write Bytes/sec";
pub const GPU_ENGINE: &str = "\\GPU Engine(*)\\Utilization Percentage";
pub const GPU_DEDICATED: &str = "\\GPU Adapter Memory(*)\\Dedicated Usage";
pub const GPU_SHARED: &str = "\\GPU Adapter Memory(*)\\Shared Usage";
/// probe の [pdh] で並べる順(§9.5)
pub const ALL: [&str; 8] = [CPU_UTILITY, CPU_TIME, DISK_IDLE, DISK_READ, DISK_WRITE, GPU_ENGINE, GPU_DEDICATED, GPU_SHARED];

struct Query(PDH_HQUERY);

impl Query {
    fn open() -> Option<Query> {
        let mut q = PDH_HQUERY::default();
        // SAFETY: データ元は実時間(null)。q はこの関数の変数
        (unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut q) } == 0).then_some(Query(q))
    }
    /// 足せなければ PDH のエラー番号
    fn add(&self, path: &str) -> Result<PDH_HCOUNTER, u32> {
        let w = wide(path);
        let mut c = PDH_HCOUNTER::default();
        // SAFETY: w は 0 で終わり、呼び出しの間生きている。c はこの関数の変数
        let r = unsafe { PdhAddEnglishCounterW(self.0, PCWSTR(w.as_ptr()), 0, &mut c) };
        if r == 0 { Ok(c) } else { Err(r) }
    }
    fn collect(&self) -> u32 {
        // SAFETY: self.0 は開いたクエリ
        unsafe { PdhCollectQueryData(self.0) }
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: self.0 は開いたクエリで、ここで 1 回だけ閉じる(カウンターも一緒に閉じる)
        unsafe {
            PdhCloseQuery(self.0);
        }
    }
}

/// 全インスタンスの (名前, 値)。値が無効なインスタンスは with_invalid が false なら外す。
/// 大きさはバイト単位で、名前の文字列も同じバッファに入る
fn array(c: PDH_HCOUNTER, with_invalid: bool) -> Option<Vec<(String, f64)>> {
    let (mut size, mut count) = (0u32, 0u32);
    // SAFETY: 大きさを聞くだけ(バッファなし)
    let mut r = unsafe { PdhGetFormattedCounterArrayW(c, FMT, &mut size, &mut count, None) };
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..4 {
        if r != PDH_MORE_DATA {
            break;
        }
        buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: buf は size バイト以上の書ける領域で、8 バイトに揃っている
        r = unsafe {
            PdhGetFormattedCounterArrayW(c, FMT, &mut size, &mut count, Some(buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()))
        };
    }
    if r != 0 {
        return if r == PDH_MORE_DATA { None } else { (count == 0).then(Vec::new) };
    }
    // SAFETY: PDH が buf の先頭に count 個の項目を書いた。名前は buf の中を指す
    let items = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(), count as usize) };
    Some(
        items
            .iter()
            .filter_map(|it| {
                let valid = it.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA || it.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA;
                if !valid && !with_invalid {
                    return None;
                }
                // SAFETY: PDH_FMT_DOUBLE で頼んだので union の中身は double
                let v = if valid { unsafe { it.FmtValue.Anonymous.doubleValue } } else { f64::NAN };
                Some((pwstr_to_string(it.szName), v))
            })
            .collect(),
    )
}

/// PDH の 1 回分
#[derive(Clone, Debug, Default)]
pub struct PdhSample {
    /// _Total の % Processor Utility(100 に収める前の値)
    pub cpu_utility_total: Option<f64>,
    pub cpu_time_total: Option<f64>,
    /// 論理プロセッサごと(グループ・番号の順。0〜100)
    pub cpu_per_core: Vec<f32>,
    pub disks: Option<Vec<DiskSample>>,
    pub gpus: Option<Vec<GpuSample>>,
    pub gpu_by_pid: GpuByPid,
}

impl PdhSample {
    /// 全体の CPU(D-9)。仮に Utility を使い(100 を超えたら 100)、取れなければ Time
    pub fn cpu_total(&self) -> Option<f32> {
        self.cpu_utility_total.or(self.cpu_time_total).map(|v| v.clamp(0.0, 100.0) as f32)
    }
}

pub struct PdhSampler {
    q: Option<Query>,
    cpu_utility: Option<PDH_HCOUNTER>,
    cpu_time: Option<PDH_HCOUNTER>,
    disk: Option<[PDH_HCOUNTER; 3]>,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_mem: Option<[PDH_HCOUNTER; 2]>,
    collected: u32,
}

impl Default for PdhSampler {
    fn default() -> Self {
        Self::open()
    }
}

impl PdhSampler {
    pub fn open() -> PdhSampler {
        let q = Query::open();
        let add = |p: &str| q.as_ref().and_then(|q| q.add(p).ok());
        let both = |a: Option<PDH_HCOUNTER>, b: Option<PDH_HCOUNTER>| Some([a?, b?]);
        let disk = match (add(DISK_IDLE), add(DISK_READ), add(DISK_WRITE)) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        };
        PdhSampler {
            cpu_utility: add(CPU_UTILITY),
            cpu_time: add(CPU_TIME),
            disk,
            gpu_engine: add(GPU_ENGINE),
            gpu_mem: both(add(GPU_DEDICATED), add(GPU_SHARED)),
            q,
            collected: 0,
        }
    }

    pub fn has_cpu(&self) -> bool {
        self.cpu_utility.is_some() || self.cpu_time.is_some()
    }
    pub fn has_disk(&self) -> bool {
        self.disk.is_some()
    }
    pub fn has_gpu(&self) -> bool {
        self.gpu_engine.is_some()
    }

    /// 集めて読む。率のカウンターは 2 回目の取得から値が出るので、1 回目は全部 None
    pub fn sample(&mut self) -> PdhSample {
        let Some(q) = &self.q else { return PdhSample::default() };
        q.collect();
        self.collected += 1;
        if self.collected < 2 {
            return PdhSample::default();
        }
        let mut s = PdhSample::default();
        let utility = self.cpu_utility.and_then(|c| array(c, false));
        let time = self.cpu_time.and_then(|c| array(c, false));
        let total = |v: &Option<Vec<(String, f64)>>| v.as_ref()?.iter().find(|(n, _)| n == "_Total").map(|(_, v)| *v);
        s.cpu_utility_total = total(&utility);
        s.cpu_time_total = total(&time);
        s.cpu_per_core = per_core(utility.as_deref().or(time.as_deref()).unwrap_or(&[]));
        if let Some([idle, read, write]) = self.disk {
            s.disks = disks(array(idle, false), array(read, false), array(write, false));
        }
        if let Some(engine) = self.gpu_engine.and_then(|c| array(c, false)) {
            let mem = self.gpu_mem.map(|[d, sh]| (array(d, false).unwrap_or_default(), array(sh, false).unwrap_or_default()));
            let (gpus, by_pid) = gpu(&engine, mem.as_ref());
            s.gpus = Some(gpus);
            s.gpu_by_pid = by_pid;
        }
        s
    }
}

/// 「グループ,番号」のインスタンスだけを番号順に(_Total と「N,_Total」は除く)
fn per_core(items: &[(String, f64)]) -> Vec<f32> {
    let mut cores: Vec<((u32, u32), f32)> = items
        .iter()
        .filter_map(|(n, v)| {
            let (g, i) = n.split_once(',')?;
            Some(((g.parse().ok()?, i.parse().ok()?), v.clamp(0.0, 100.0) as f32))
        })
        .collect();
    cores.sort_by_key(|(k, _)| *k);
    cores.into_iter().map(|(_, v)| v).collect()
}

fn disks(
    idle: Option<Vec<(String, f64)>>,
    read: Option<Vec<(String, f64)>>,
    write: Option<Vec<(String, f64)>>,
) -> Option<Vec<DiskSample>> {
    let (read, write): (HashMap<_, _>, HashMap<_, _>) = (read?.into_iter().collect(), write?.into_iter().collect());
    let mut out: Vec<DiskSample> = idle?
        .into_iter()
        .filter(|(n, _)| n != "_Total")
        .map(|(name, idle)| DiskSample {
            active_pct: (100.0 - idle).clamp(0.0, 100.0) as f32,
            read_bps: read.get(&name).copied().unwrap_or(0.0).max(0.0),
            write_bps: write.get(&name).copied().unwrap_or(0.0).max(0.0),
            name,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Some(out)
}

/// GPU Engine のインスタンス名(例: pid_1234_luid_0x00000000_0x0000D1A3_phys_0_eng_0_engtype_3D)を (PID, luid, engtype) に
pub fn parse_engine(name: &str) -> Option<(u32, &str, &str)> {
    let rest = name.strip_prefix("pid_")?;
    let pid = rest.split('_').next()?.parse().ok()?;
    let luid = adapter_luid(&name[name.find("luid_")?..])?;
    let engtype = &name[name.rfind("engtype_")? + "engtype_".len()..];
    Some((pid, luid, engtype))
}

/// 「luid_0x..._0x...」の部分(後ろの _phys_N などを外す)
fn adapter_luid(s: &str) -> Option<&str> {
    let s = &s[s.find("luid_")?..];
    let end = s.find("_phys").or_else(|| s.find("_eng_")).unwrap_or(s.len());
    Some(&s[..end])
}

/// D-12・D-13。engtype ごとに足し、その最大を取る(100 を超えたら 100)。PID ごととアダプター(luid)ごと
fn gpu(engine: &[(String, f64)], mem: Option<&(Items, Items)>) -> (Vec<GpuSample>, GpuByPid) {
    let mut by_pid: HashMap<(u32, &str), f64> = HashMap::new();
    let mut by_luid: HashMap<(&str, &str), f64> = HashMap::new();
    let mut luids: BTreeMap<String, (f32, Option<u64>, Option<u64>)> = BTreeMap::new();
    for (name, v) in engine {
        let Some((pid, luid, eng)) = parse_engine(name) else { continue };
        *by_pid.entry((pid, eng)).or_default() += v;
        *by_luid.entry((luid, eng)).or_default() += v;
        luids.entry(luid.to_string()).or_default();
    }
    let mut pid_max = GpuByPid::new();
    for ((pid, _), v) in by_pid {
        let e = pid_max.entry(pid).or_insert(0.0);
        *e = e.max(v.min(100.0) as f32);
    }
    for ((luid, _), v) in by_luid {
        let e = &mut luids.get_mut(luid).expect("上で入れた").0;
        *e = e.max(v.min(100.0) as f32);
    }
    if let Some((ded, sh)) = mem {
        for (list, dedicated) in [(ded, true), (sh, false)] {
            for (name, v) in list {
                let Some(luid) = adapter_luid(name) else { continue };
                let e = luids.entry(luid.to_string()).or_default();
                let slot = if dedicated { &mut e.1 } else { &mut e.2 };
                *slot = Some(slot.unwrap_or(0) + v.max(0.0) as u64);
            }
        }
    }
    // アダプターの名前は取らない(DXGI を使わない)。luid の順に「GPU 0」「GPU 1」…
    let gpus = luids
        .into_iter()
        .enumerate()
        .map(|(i, (luid, (util, d, s)))| GpuSample { luid, name: format!("GPU {i}"), util_pct: util, dedicated_bytes: d, shared_bytes: s })
        .collect();
    (gpus, pid_max)
}

/// probe の [pdh]。カウンターを 1 本ずつ新しいクエリに足し、2 回集めてインスタンス名を返す。足せなければ PDH のエラー番号
pub fn probe_counters() -> Vec<(&'static str, Result<Vec<String>, u32>)> {
    let Some(q) = Query::open() else { return ALL.iter().map(|p| (*p, Err(u32::MAX))).collect() };
    let added: Vec<_> = ALL.iter().map(|p| (*p, q.add(p))).collect();
    q.collect();
    std::thread::sleep(std::time::Duration::from_millis(250));
    q.collect();
    added
        .into_iter()
        .map(|(p, c)| (p, c.map(|c| array(c, true).unwrap_or_default().into_iter().map(|(n, _)| n).collect())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_names() {
        let n = "pid_1234_luid_0x00000000_0x0000D1A3_phys_0_eng_3_engtype_VideoDecode";
        assert_eq!(parse_engine(n), Some((1234, "luid_0x00000000_0x0000D1A3", "VideoDecode")));
        assert_eq!(parse_engine("luid_0x0_0x1_phys_0"), None);
        assert_eq!(adapter_luid("luid_0x00000000_0x0000D1A3_phys_0"), Some("luid_0x00000000_0x0000D1A3"));
    }

    #[test]
    fn gpu_sums_engtype_then_max() {
        let e = |pid: u32, eng: u32, t: &str, v: f64| {
            (format!("pid_{pid}_luid_0x00000000_0x0000AAAA_phys_0_eng_{eng}_engtype_{t}"), v)
        };
        let engine = vec![e(10, 0, "3D", 30.0), e(10, 1, "3D", 40.0), e(10, 2, "Copy", 50.0), e(20, 0, "3D", 80.0)];
        let mem = (vec![("luid_0x00000000_0x0000AAAA_phys_0".to_string(), 1000.0)], vec![]);
        let (gpus, by_pid) = gpu(&engine, Some(&mem));
        assert_eq!(by_pid[&10], 70.0);
        assert_eq!(by_pid[&20], 80.0);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].util_pct, 100.0, "3D の合計 150 → 100");
        assert_eq!(gpus[0].dedicated_bytes, Some(1000));
        assert_eq!(gpus[0].shared_bytes, None);
    }

    #[test]
    fn cores_skip_totals() {
        let items: Vec<(String, f64)> =
            [("_Total", 1.0), ("0,_Total", 2.0), ("0,1", 30.0), ("0,0", 120.0), ("1,0", 5.0)].iter().map(|(n, v)| (n.to_string(), *v)).collect();
        assert_eq!(per_core(&items), vec![100.0, 30.0, 5.0]);
    }

    #[test]
    fn real_pdh_cpu() {
        let mut s = PdhSampler::open();
        assert!(s.has_cpu());
        let _ = s.sample();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let v = s.sample();
        assert!(v.cpu_total().is_some());
        assert_eq!(v.cpu_per_core.len(), crate::util::logical_cpus() as usize);
    }
}
