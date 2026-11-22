//! Worker スレッド × 2(担当 A)。Job を 1 つずつ処理して JobResult を返す。操作の前に Core の rules を通す。
//! 画面の Job を先に、StaticInfo の取得は空いているときに行う。Job の処理が panic しても Worker は止まらず、失敗の結果を返す。
//! プロセスに触れる Job の前には、触れない集合を今の SPI で作り直す(INV-1)。

use crate::collector::{Shared, lock};
use crate::details::{self, SignatureCache};
use crate::gate::GateError;
use crate::jobs::{Job, JobResult, OpError, SigStatus, Signature, Target};
use crate::{elevation, icons, ops, services, shell, startup, static_info, taskmgr_replace};
use procsight_core::ProcId;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock};

enum Task {
    Job(Job),
    Static(ProcId, String),
}

/// 署名の覚えは 2 本の Worker で共有する(D-23)。作るのは最初に要ったとき
pub(crate) type SigCell = Arc<OnceLock<SignatureCache>>;

pub(crate) fn run(shared: Arc<Shared>, sig: SigCell) {
    loop {
        let task = {
            let mut q = lock(&shared.queue);
            loop {
                if q.stop {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    break Task::Job(j);
                }
                if let Some((id, name)) = q.statics.pop_front() {
                    break Task::Static(id, name);
                }
                q = shared.queue_cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        match task {
            Task::Job(job) => {
                let results = catch_unwind(AssertUnwindSafe(|| handle(&shared, &sig, job.clone())))
                    .unwrap_or_else(|_| vec![failed(&job, OpError::Unsupported)]);
                let tx = lock(&shared.results).clone();
                for r in results {
                    let _ = tx.send(r);
                }
                (shared.repaint)();
            }
            Task::Static(id, name) => {
                // panic しても Worker を減らさない
                if catch_unwind(AssertUnwindSafe(|| fetch_static(&shared, id, &name))).is_err() {
                    lock(&shared.queue).requested.remove(&id);
                }
            }
        }
    }
}

fn fetch_static(shared: &Shared, id: ProcId, name: &str) {
    let lower = name.to_lowercase();
    let names_hit = lock(&shared.ctl).touch.names.iter().any(|n| n.to_lowercase() == lower);
    // 待っている間に終わった・触れないになった物は開かない(PID が触れないプロセスに使い回される隙を狭める)
    let snap = shared.current();
    let live = snap.find(id).is_some_and(|r| r.is_gone == 0 && !r.touch_free);
    if names_hit || !live || shared.gate.is_touch_free(id) {
        lock(&shared.queue).requested.remove(&id);
        return;
    }
    let info = static_info::fetch(&shared.gate, id);
    if info.error == Some(procsight_core::InfoError::TouchFree) {
        lock(&shared.queue).requested.remove(&id);
        return;
    }
    lock(&shared.statics).insert(id, Arc::new(info));
}

/// その Job が開くプロセス(1 つの物だけ)
fn single_id(job: &Job) -> Option<ProcId> {
    match job {
        Job::SetPriority { target, .. }
        | Job::GetAffinity { target }
        | Job::SetAffinity { target, .. }
        | Job::SetEfficiency { target, .. }
        | Job::Suspend { target }
        | Job::Resume { target }
        | Job::BringToFront { target } => Some(target.id),
        Job::General { id } | Job::Threads { id } | Job::Modules { id } | Job::Handles { id } => Some(*id),
        _ => None,
    }
}

/// 触れないプロセスの exe のファイルを読まない(INV-1。アイコン・署名・場所を開く・プロパティ)。
/// 登録した名前と、今触れないとしているプロセス(子孫として触れない物を含む)の exe 名のどれかと一致するか
fn path_is_touch_free(shared: &Shared, path: &str) -> bool {
    let file = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    lock(&shared.ctl).touch.names.iter().any(|n| n.to_lowercase() == file)
        || shared.current().rows.iter().any(|r| r.touch_free && r.name.to_lowercase() == file)
}

fn op(job: Job, result: Result<(), OpError>) -> Vec<JobResult> {
    vec![JobResult::Op { job, result }]
}

fn handle(shared: &Arc<Shared>, sig: &SigCell, job: Job) -> Vec<JobResult> {
    let gate = &shared.gate;
    let touches_process = single_id(&job).is_some()
        || matches!(job, Job::Terminate { .. } | Job::ResumeMany { .. } | Job::OpenTaskManager | Job::FindLocking { .. });
    let touch_free = if touches_process { shared.refresh_touch_free() } else { Default::default() };
    if let Some(id) = single_id(&job)
        && gate.is_touch_free(id)
    {
        return vec![failed(&job, OpError::Gate(GateError::TouchFree))];
    }
    let (me, root) = (shared.self_pid, shared.system_root.as_str());
    match job {
        Job::Terminate { targets, tree } => {
            let snap = shared.current();
            let results = targets
                .into_iter()
                .map(|t| {
                    let r = ops::check_row(&snap, t.id, false, me, root).and_then(|_| ops::terminate(gate, t.id));
                    (t, r)
                })
                .collect();
            vec![JobResult::Terminated { results, tree }]
        }
        Job::SetPriority { ref target, priority } => {
            let r = ops::set_priority(gate, target.id, priority);
            op(job, r)
        }
        Job::GetAffinity { target } => {
            let result = ops::get_affinity(gate, target.id);
            vec![JobResult::Affinity { target, result }]
        }
        Job::SetAffinity { ref target, mask } => {
            let r = ops::set_affinity(gate, target.id, mask);
            op(job, r)
        }
        Job::SetEfficiency { ref target, on } => {
            let r = ops::set_efficiency(gate, target.id, on);
            op(job, r)
        }
        Job::Suspend { ref target } => {
            let r = ops::check_row(&shared.current(), target.id, true, me, root).and_then(|_| ops::suspend(gate, target.id));
            op(job, r)
        }
        Job::Resume { ref target } => {
            let r = ops::resume(gate, target.id);
            op(job, r)
        }
        Job::ResumeMany { ref targets } => {
            let mut r = Ok(());
            for t in targets {
                if let Err(e) = ops::resume(gate, t.id)
                    && r.is_ok()
                {
                    r = Err(e);
                }
            }
            op(job, r)
        }
        Job::BringToFront { ref target } => {
            let r = ops::bring_to_front(gate, target);
            op(job, r)
        }
        // エクスプローラーがその exe を読むので、触れないプロセスの exe なら開かない(INV-1)
        Job::OpenLocation { ref path } | Job::ShowProperties { ref path } if path_is_touch_free(shared, path) => {
            op(job, Err(OpError::Gate(GateError::TouchFree)))
        }
        Job::OpenLocation { ref path } => {
            let r = shell::open_location(path);
            op(job, r)
        }
        Job::ShowProperties { ref path } => {
            let r = shell::show_properties(path);
            op(job, r)
        }
        Job::OpenTaskManager => {
            let (r, restore_failed) = taskmgr_replace::open_task_manager(gate);
            let mut out = op(job, r);
            if let Some(saved) = restore_failed {
                out.push(JobResult::ReplaceRestoreFailed { saved });
            }
            out
        }
        Job::General { id } => vec![JobResult::General { id, info: details::general(gate, id) }],
        Job::Threads { id } => vec![JobResult::Threads { id, result: threads_of(id) }],
        Job::Modules { id } => vec![JobResult::Modules { id, result: details::modules(gate, id) }],
        Job::Handles { id } => vec![JobResult::Handles { id, result: details::handles(gate, id) }],
        Job::Signature { path } => {
            let result = if path_is_touch_free(shared, &path) {
                refused_signature()
            } else {
                sig.get_or_init(SignatureCache::new).check(&path)
            };
            vec![JobResult::Signature { path, result }]
        }
        Job::FindLocking { paths } => {
            // D-25: RmGetList が中で他のプロセスを開くか分からないので、触れないプロセスがいる間は呼ばない
            let result = if touch_free.is_empty() {
                details::find_locking(&paths)
            } else {
                Err(OpError::Blocked("触れないプロセスが動いている間は、この検索を使えません"))
            };
            vec![JobResult::Locking { paths, result }]
        }
        Job::LoadStartup => {
            let names = lock(&shared.ctl).touch.names.clone();
            vec![JobResult::Startup { result: startup::load(sig.get_or_init(SignatureCache::new), &names) }]
        }
        Job::SetStartup { name, location, enabled } => {
            let result = startup::set(&name, location, enabled);
            vec![JobResult::StartupSet { name, enabled, result }]
        }
        Job::LoadServices => vec![JobResult::Services { result: services::load() }],
        Job::ServiceDependents { name } => {
            let result = services::dependents(&name);
            vec![JobResult::ServiceDependents { name, result }]
        }
        Job::ServiceControl { name, display_name, action, confirmed } => {
            let result = services::control(&name, action, &confirmed);
            vec![JobResult::ServiceDone { display_name, action, result }]
        }
        Job::SetStartType { name, display_name, start_type } => {
            let result = services::set_start_type(&name, start_type);
            vec![JobResult::StartTypeSet { display_name, start_type, result }]
        }
        Job::Icon { path } => {
            let rgba = if path_is_touch_free(shared, &path) { None } else { icons::icon_rgba(&path) };
            vec![JobResult::Icon { path, rgba }]
        }
        Job::RestartElevated => vec![JobResult::RestartElevated { result: elevation::restart_elevated() }],
        Job::ReplaceStatus => vec![JobResult::Replace { info: taskmgr_replace::read_info() }],
        Job::ReplaceEnable => {
            let (previous, result) = match taskmgr_replace::enable() {
                Ok(p) => (p, Ok(())),
                Err(e) => (String::new(), Err(e)),
            };
            vec![JobResult::ReplaceChanged { enabled: true, previous, result }]
        }
        Job::ReplaceDisable { previous } => {
            let result = taskmgr_replace::disable(&previous);
            vec![JobResult::ReplaceChanged { enabled: false, previous, result }]
        }
        Job::ReplaceRestore { value } => vec![JobResult::ReplaceRestored { result: taskmgr_replace::restore(&value) }],
        Job::PickFiles => vec![JobResult::PickedFiles { paths: shell::pick_files() }],
    }
}

/// D-20。SPI のスレッドの項目から作る(ハンドルを開かない)
fn threads_of(id: ProcId) -> Result<Vec<procsight_core::RawThread>, OpError> {
    let ps = crate::nt::spi::query_processes().map_err(|s| OpError::Win32(s as u32))?;
    ps.into_iter().find(|p| p.id == id).map(|p| p.threads).ok_or(OpError::Gate(GateError::Gone))
}

fn refused_signature() -> Signature {
    Signature {
        status: SigStatus::Failed(OpError::Gate(GateError::TouchFree).message(false)),
        signer: None,
        checked_at: crate::util::now_filetime(),
    }
}

fn all_failed(targets: &[Target], e: &OpError) -> Vec<(Target, Result<(), OpError>)> {
    targets.iter().map(|t| (t.clone(), Err(e.clone()))).collect()
}

/// Job が失敗したときの結果(断ったとき・処理が panic したとき)
pub(crate) fn failed(job: &Job, e: OpError) -> JobResult {
    let job = job.clone();
    match job {
        Job::Terminate { targets, tree } => JobResult::Terminated { results: all_failed(&targets, &e), tree },
        Job::GetAffinity { target } => JobResult::Affinity { target, result: Err(e) },
        Job::General { id } => JobResult::General { id, info: Err(e) },
        Job::Modules { id } => JobResult::Modules { id, result: Err(e) },
        Job::Handles { id } => JobResult::Handles { id, result: Err(e) },
        Job::Signature { path } => JobResult::Signature {
            path,
            result: Signature { status: SigStatus::Failed(e.message(false)), signer: None, checked_at: crate::util::now_filetime() },
        },
        Job::FindLocking { paths } => JobResult::Locking { paths, result: Err(e) },
        Job::LoadStartup => JobResult::Startup { result: Err(e) },
        Job::SetStartup { name, enabled, .. } => JobResult::StartupSet { name, enabled, result: Err(e) },
        Job::LoadServices => JobResult::Services { result: Err(e) },
        Job::ServiceDependents { name } => JobResult::ServiceDependents { name, result: Err(e) },
        Job::ServiceControl { display_name, action, .. } => JobResult::ServiceDone { display_name, action, result: Err(e) },
        Job::SetStartType { display_name, start_type, .. } => JobResult::StartTypeSet { display_name, start_type, result: Err(e) },
        Job::Icon { path } => JobResult::Icon { path, rgba: None },
        Job::RestartElevated => JobResult::RestartElevated { result: Err(e) },
        // ReplaceStatus は値の無い ReplaceInfo を返すと「置き換えなし」に見えるので、Op の失敗で返す
        Job::ReplaceEnable => JobResult::ReplaceChanged { enabled: true, previous: String::new(), result: Err(e) },
        Job::ReplaceDisable { previous } => JobResult::ReplaceChanged { enabled: false, previous, result: Err(e) },
        Job::PickFiles => JobResult::PickedFiles { paths: Vec::new() },
        Job::ReplaceRestore { .. } => JobResult::ReplaceRestored { result: Err(e) },
        job => JobResult::Op { job, result: Err(e) },
    }
}
