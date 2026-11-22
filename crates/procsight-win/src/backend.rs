//! 画面から見た裏の処理の入口(担当 A)。Collector・Worker・ProcessGate を起こし、Snapshot と JobResult を渡す。
//! 画面はこの型の関数だけを呼ぶ(INV-9)。起動時に SPI の非公開の位置を自分のプロセスで確かめ、合わない項目は使わない。

use crate::collector::{self, Control, DiagState, Queue, Shared, lock};
use crate::gate::ProcessGate;
use crate::jobs::{Job, JobResult};
use crate::nt::{spi, suspend};
use crate::{static_info, util, worker};
use procsight_core::Snapshot;
use procsight_core::rules::TouchFreeSet;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub interval_ms: u64,
    pub touch_free: TouchFreeSet,
}

/// 公開されていない API と PDH の使える・使えない(画面設計書 §7.4)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApiAvailability {
    /// SPI の CPU 時間・開始時刻(事実19)
    pub spi_times: bool,
    /// SPI のプライベート作業セット(事実19)
    pub spi_private_ws: bool,
    /// SPI の I/O の量(事実19)
    pub spi_io: bool,
    /// NtSuspendProcess / NtResumeProcess(事実21)
    pub suspend: bool,
    /// ProcessCommandLineInformation(事実23)
    pub cmdline: bool,
    /// SystemExtendedHandleInformation(事実22)
    pub handles: bool,
    pub pdh_cpu: bool,
    pub pdh_disk: bool,
    pub pdh_gpu: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub api: ApiAvailability,
    /// Collector の 1 周の時間(ミリ秒)の直近 200 回。時間の記録がオンのときだけ溜まる
    pub collect_ms: Vec<f32>,
    /// 更新間隔に間に合わなかった周が続いた回数(E-38。3 で帯を出す)
    pub late_in_row: u32,
    /// SPI の取得が続けて失敗した回数(画面設計書 §11。4 で帯を出す)
    pub spi_fail_in_row: u32,
    pub open_counts: Vec<(u32, u32)>,
    pub refused_touch_free: u64,
    pub timing: bool,
}

pub struct Backend {
    shared: Arc<Shared>,
    rx: Mutex<Receiver<JobResult>>,
    collector: Option<JoinHandle<()>>,
}

impl Backend {
    /// Collector 1 本、Worker 2 本を起こす。`repaint` は新しい Snapshot・JobResult ができたときに呼ぶ(画面を描き直させる)
    pub fn start(cfg: BackendConfig, repaint: Arc<dyn Fn() + Send + Sync>) -> Backend {
        let check = spi::self_check();
        spi::set_fields(check.fields);
        let api = ApiAvailability {
            // CreateTime が合わなければ識別子が作れないので、CPU 時間も使わない
            spi_times: check.create_time_ok && check.fields.cpu_times,
            spi_private_ws: check.fields.private_ws,
            spi_io: check.fields.io,
            suspend: suspend::available(),
            // 自分のプロセスで取れるかを確かめる(D-6)。取れなくても隠すだけで動き続ける
            cmdline: crate::nt::cmdline::self_check(),
            handles: crate::nt::handles::self_check(),
            // PDH は Collector のスレッドで開いてから入れる
            pdh_cpu: false,
            pdh_disk: false,
            pdh_gpu: false,
        };
        let self_pid = util::self_pid();
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            gate: Arc::new(ProcessGate::new(self_pid, static_info::self_elevated())),
            snapshot: Mutex::new(Arc::new(Snapshot::default())),
            ctl: Mutex::new(Control {
                interval_ms: cfg.interval_ms,
                paused: false,
                minimized: false,
                touch: cfg.touch_free,
                timing: false,
                stop: false,
                wake: false,
            }),
            ctl_cv: Condvar::new(),
            queue: Mutex::new(Queue::default()),
            queue_cv: Condvar::new(),
            statics: Mutex::new(Default::default()),
            results: Mutex::new(tx),
            diag: Mutex::new(DiagState::default()),
            touch_pids: Mutex::new(Default::default()),
            api: Mutex::new(api),
            repaint,
            self_pid,
            system_root: util::system_root(),
        });
        let sig: worker::SigCell = Arc::new(OnceLock::new());
        for i in 0..2 {
            let (s, sig) = (shared.clone(), sig.clone());
            let _ = std::thread::Builder::new().name(format!("procsight-worker-{i}")).spawn(move || worker::run(s, sig));
        }
        let s = shared.clone();
        // Collector が panic したら 1 秒置いて起こし直す(Snapshot と関門の触れない集合の更新を止めない)。止める印で正常に戻ったら終わる
        let collector = std::thread::Builder::new()
            .name("procsight-collector".into())
            .spawn(move || {
                while std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| collector::run(s.clone()))).is_err() {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            })
            .ok();
        Backend { shared, rx: Mutex::new(rx), collector }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared.current()
    }

    pub fn submit(&self, job: Job) {
        lock(&self.shared.queue).jobs.push_back(job);
        self.shared.queue_cv.notify_one();
    }

    pub fn try_recv(&self) -> Option<JobResult> {
        lock(&self.rx).try_recv().ok()
    }

    fn control(&self, f: impl FnOnce(&mut Control)) {
        f(&mut lock(&self.shared.ctl));
        self.shared.ctl_cv.notify_all();
    }

    pub fn set_interval(&self, ms: u64) {
        self.control(|c| c.interval_ms = ms);
    }
    /// FR-49。Collector を止める(グラフも止まる)
    pub fn set_paused(&self, paused: bool) {
        self.control(|c| c.paused = paused);
    }
    /// D-36。最小化・隠している間はプロセスごとの値を作らない
    pub fn set_minimized(&self, minimized: bool) {
        // 戻したら待たずに 1 周して、古い行を早く替える
        self.control(|c| {
            c.minimized = minimized;
            c.wake |= !minimized;
        });
    }
    /// 待たずに 1 周して、関門と Snapshot の触れない集合を作り直す
    pub fn set_touch_free(&self, set: TouchFreeSet) {
        self.control(|c| {
            c.touch = set;
            c.wake = true;
        });
    }
    /// 診断の「時間を記録する」(§9.8)
    pub fn set_timing(&self, on: bool) {
        self.control(|c| c.timing = on);
        if !on {
            lock(&self.shared.diag).collect_ms.clear();
        }
    }
    pub fn diagnostics(&self) -> Diagnostics {
        let d = lock(&self.shared.diag);
        Diagnostics {
            api: lock(&self.shared.api).clone(),
            collect_ms: d.collect_ms.iter().copied().collect(),
            late_in_row: d.late_in_row,
            spi_fail_in_row: d.spi_fail_in_row,
            open_counts: self.shared.gate.open_counts(),
            refused_touch_free: self.shared.gate.refused_touch_free(),
            timing: lock(&self.shared.ctl).timing,
        }
    }
    pub fn gate(&self) -> &Arc<ProcessGate> {
        &self.shared.gate
    }
    /// スレッドを止める。戻り値は (触れないプロセスへの開いた回数, 開いた回数の合計)(§9.8 の gate_summary)。
    /// Worker は長い仕事(サービスの 30 秒・ファイルを選ぶ窓)の途中かもしれないので待たない。止める印だけを付ける
    pub fn shutdown(mut self) -> (u64, u64) {
        self.control(|c| c.stop = true);
        lock(&self.shared.queue).stop = true;
        self.shared.queue_cv.notify_all();
        if let Some(h) = self.collector.take() {
            let _ = h.join();
        }
        // D-40 で Debugger を外している最中なら、戻し終わるまで最大 6 秒待つ(外したまま終わらない)
        let until = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while crate::taskmgr_replace::swap_in_progress() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let pids = lock(&self.shared.touch_pids).clone();
        self.shared.gate.summary(&pids)
    }
}
