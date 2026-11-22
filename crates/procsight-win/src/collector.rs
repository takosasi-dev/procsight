//! Collector スレッド(担当 A)。更新間隔ごとに SPI・PDH・ネット・メモリ・窓の一覧を読み、Core の diff で Snapshot を作る。
//! どのプロセスのハンドルも開かない(INV-2)。新しい識別子は StaticInfo の取得を Worker に頼む(触れない物は頼まない)。
//! 1 周が更新間隔を超えたら次の周を飛ばす(E-38)。最小化の間は全体の値だけを取る(D-36)。

use crate::backend::ApiAvailability;
use crate::gate::ProcessGate;
use crate::jobs::{Job, JobResult};
use crate::nt::spi::{self, SpiReader};
use crate::pdh::PdhSampler;
use crate::{mem, net, util, window_scan};
use procsight_core::rules::TouchFreeSet;
use procsight_core::{GpuByPid, PerfSample, ProcId, RawSnapshot, Snapshot, StaticCache, diff};
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 診断の時間を溜める数(§9.8・Diagnostics)
const TIMING_KEEP: usize = 200;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) struct Control {
    pub interval_ms: u64,
    pub paused: bool,
    pub minimized: bool,
    pub touch: TouchFreeSet,
    pub timing: bool,
    pub stop: bool,
    /// 待たずに 1 周する(触れない集合が変わったとき)
    pub wake: bool,
}

/// Worker の仕事。画面の Job を先に、StaticInfo の取得は空いたときに
#[derive(Default)]
pub(crate) struct Queue {
    pub jobs: VecDeque<Job>,
    /// (識別子, SPI の名前)
    pub statics: VecDeque<(ProcId, String)>,
    pub requested: HashSet<ProcId>,
    pub stop: bool,
}

#[derive(Default)]
pub(crate) struct DiagState {
    pub collect_ms: VecDeque<f32>,
    pub late_in_row: u32,
    pub spi_fail_in_row: u32,
}

pub(crate) struct Shared {
    pub gate: Arc<ProcessGate>,
    pub snapshot: Mutex<Arc<Snapshot>>,
    pub ctl: Mutex<Control>,
    pub ctl_cv: Condvar,
    pub queue: Mutex<Queue>,
    pub queue_cv: Condvar,
    pub statics: Mutex<StaticCache>,
    pub results: Mutex<Sender<JobResult>>,
    pub diag: Mutex<DiagState>,
    /// 今までに触れないとされた PID の全部(終了時の gate_summary)
    pub touch_pids: Mutex<HashSet<u32>>,
    pub api: Mutex<ApiAvailability>,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    pub self_pid: u32,
    pub system_root: String,
}

impl Shared {
    pub fn note_touch_free(&self, ids: &HashSet<ProcId>) {
        lock(&self.touch_pids).extend(ids.iter().map(|i| i.pid));
    }

    /// 触れない集合を今の SPI で作り直して関門に入れる(Worker が操作の直前に呼ぶ)。
    /// SPI が取れなければ今の Snapshot の集合を返し、関門の集合は狭めない
    pub fn refresh_touch_free(&self) -> HashSet<ProcId> {
        let mut touch = lock(&self.ctl).touch.clone();
        // 今の Snapshot で触れないとした物は残す(狭めない側に倒す)
        touch.keep = lock(&self.snapshot).touch_free.clone();
        let read_at = util::tick_ms();
        match spi::query_processes() {
            Ok(ps) => {
                let ids = touch.resolve(&RawSnapshot { processes: ps, ..Default::default() });
                self.gate.set_touch_free_at(ids.clone(), read_at);
                self.note_touch_free(&ids);
                ids
            }
            Err(_) => lock(&self.snapshot).touch_free.clone(),
        }
    }

    pub fn current(&self) -> Arc<Snapshot> {
        lock(&self.snapshot).clone()
    }

    fn publish(&self, s: Snapshot) {
        *lock(&self.snapshot) = Arc::new(s);
        (self.repaint)();
    }
}

/// 1 周の取得(SPI・窓・diff と、全体の値)。Collector と probe の時間の測定で使う
pub struct Sampler {
    spi: SpiReader,
    pdh: PdhSampler,
    net: net::NetSampler,
    logical_cpus: u32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler { spi: SpiReader::new(), pdh: PdhSampler::open(), net: net::NetSampler::default(), logical_cpus: util::logical_cpus() }
    }

    pub fn pdh(&self) -> &PdhSampler {
        &self.pdh
    }

    /// 全体の値と、PID ごとの GPU
    pub fn perf(&mut self) -> (PerfSample, GpuByPid) {
        let p = self.pdh.sample();
        let perf = PerfSample {
            cpu_total: p.cpu_total(),
            cpu_per_core: p.cpu_per_core.clone(),
            mem: mem::sample(),
            disks: p.disks,
            nets: self.net.sample(),
            gpus: p.gpus,
            uptime_ms: util::tick_ms(),
        };
        (perf, p.gpu_by_pid)
    }

    /// プロセスの行(NFR-3 の「一覧の取得と差分の計算」)。prev が None なら CPU と I/O は「—」(E-10・D-36)
    pub fn rows(
        &mut self,
        prev: Option<&Snapshot>,
        gpu: &GpuByPid,
        touch: &TouchFreeSet,
        statics: &StaticCache,
    ) -> Result<Snapshot, i32> {
        let processes = self.spi.read(spi::fields())?;
        let raw = RawSnapshot { taken_at_ms: util::tick_ms(), logical_cpus: self.logical_cpus, processes };
        let windows = window_scan::scan();
        Ok(diff::diff(prev, raw, gpu, &windows, touch, statics))
    }
}

pub(crate) fn run(shared: Arc<Shared>) {
    let mut sampler = Sampler::new();
    {
        let mut api = lock(&shared.api);
        api.pdh_cpu = sampler.pdh.has_cpu();
        api.pdh_disk = sampler.pdh.has_disk();
        api.pdh_gpu = sampler.pdh.has_gpu();
    }
    let mut next = Instant::now();
    // 最小化から戻った最初の 1 回は古い累計との差を取らない(D-36)
    let mut after_minimized = false;
    // 前の周の触れないの登録(変わったら keep を引き継がない)
    let mut last_rules: Option<TouchFreeSet> = None;
    loop {
        let (interval, minimized, touch, timing) = {
            let mut c = lock(&shared.ctl);
            loop {
                if c.stop {
                    return;
                }
                if c.wake && !c.paused {
                    c.wake = false;
                    break;
                }
                let now = Instant::now();
                if c.paused {
                    c = shared.ctl_cv.wait(c).unwrap_or_else(|e| e.into_inner());
                } else if now >= next {
                    break;
                } else {
                    c = shared.ctl_cv.wait_timeout(c, next - now).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
            (Duration::from_millis(c.interval_ms.max(100)), c.minimized, c.touch.clone(), c.timing)
        };
        let start = Instant::now();
        let prev = shared.current();
        let (perf, gpu) = sampler.perf();
        let stale = |prev: &Snapshot, perf: PerfSample| {
            let mut s = prev.clone();
            s.seq = prev.seq + 1;
            s.perf = perf;
            s.rows_stale = true;
            s
        };
        if minimized {
            after_minimized = true;
            shared.publish(stale(&prev, perf));
        } else {
            let base = (prev.seq > 0 && !after_minimized).then_some(&*prev);
            // 登録が変わっていなければ、前の周に触れないとした物を残す(ランチャーが終わった後のゲームなど)
            let mut touch = touch;
            if last_rules.as_ref().is_some_and(|r| r.same_rules(&touch)) {
                touch.keep = prev.touch_free.clone();
            }
            last_rules = Some(touch.clone());
            // 関門の集合の新しさは、SPI を読み始めた時刻で比べる
            let read_at = util::tick_ms();
            let rows = {
                let statics = lock(&shared.statics);
                sampler.rows(base, &gpu, &touch, &statics)
            };
            match rows {
                Ok(mut snap) => {
                    lock(&shared.diag).spi_fail_in_row = 0;
                    after_minimized = false;
                    snap.seq = prev.seq + 1;
                    snap.perf = perf;
                    // 公開の前に関門へ(画面が見る集合と関門の集合を同じにする。FR-22)
                    shared.gate.set_touch_free_at(snap.touch_free.clone(), read_at);
                    shared.note_touch_free(&snap.touch_free);
                    // 公開してから頼む(Worker が今の Snapshot で生きているかを確かめるため)
                    shared.publish(snap);
                    let wanted = request_statics(&shared, &shared.current());
                    if wanted {
                        shared.queue_cv.notify_all();
                    }
                }
                Err(_) => {
                    // E-9: その周の行は作らない(画面設計書 §11 の帯は spi_fail_in_row で)
                    lock(&shared.diag).spi_fail_in_row += 1;
                    shared.publish(stale(&prev, perf));
                }
            }
        }
        let took = start.elapsed();
        let late = took > interval;
        // E-38: 間に合わなければ次の周を飛ばす(溜めない)
        next = start + if late { interval * 2 } else { interval };
        let mut d = lock(&shared.diag);
        d.late_in_row = if late { d.late_in_row + 1 } else { 0 };
        if timing {
            d.collect_ms.push_back(took.as_secs_f32() * 1000.0);
            while d.collect_ms.len() > TIMING_KEEP {
                d.collect_ms.pop_front();
            }
        }
    }
}

/// 新しい識別子の StaticInfo を Worker に頼む(触れない物・終わった物は頼まない)。いなくなった識別子は入れ物から消す
fn request_statics(shared: &Shared, snap: &Snapshot) -> bool {
    let present: HashSet<ProcId> = snap.rows.iter().filter(|r| r.is_gone == 0).map(|r| r.id).collect();
    let mut statics = lock(&shared.statics);
    statics.retain(|id, _| present.contains(id));
    let mut q = lock(&shared.queue);
    q.requested.retain(|id| present.contains(id));
    let mut wanted = false;
    for r in snap.rows.iter().filter(|r| r.is_gone == 0 && !r.touch_free && r.id.pid != 0) {
        if !statics.contains_key(&r.id) && q.requested.insert(r.id) {
            q.statics.push_back((r.id, r.name.clone()));
            wanted = true;
        }
    }
    wanted
}
