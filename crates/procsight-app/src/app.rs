//! 画面の状態(State)と、窓の共通の部分(メニュー・権限の印・タブ・更新間隔・一時停止・帯・状態の行)。
//! State は Backend を持たない。画面は Action を積み、JobResult を `apply` で受けるだけ(INV-9)。
//! 終了・一時停止・サービスの停止・開始の種類の Job は dialogs.rs の実行のボタンだけが積む(INV-5)。

use eframe::egui;
use crate::dialogs::Dialog;
use crate::files::SuspendedEntry;
use crate::fonts::LoadedFont;
use crate::theme::{self, Palette};
use crate::{details, dialogs, locks, perf, procs, services_view, settings, startup_view};
use egui::{Align, Color32, Key, Layout, Modifiers, RichText, Ui};
use procsight_core::config::{Config, INTERVALS_MS};
use procsight_core::format;
use procsight_core::{ProcId, ProcState, Snapshot};
use procsight_win::jobs::{Job, JobResult, OpError, ReplaceInfo, Signature, Target};
use procsight_win::{Diagnostics, EnvInfo};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Processes,
    Performance,
    Startup,
    Services,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Processes, Tab::Performance, Tab::Startup, Tab::Services];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Processes => "プロセス",
            Tab::Performance => "パフォーマンス",
            Tab::Startup => "スタートアップ",
            Tab::Services => "サービス",
        }
    }
}

/// 画面から殻(main の eframe::App)への頼み事
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Job(Job),
    SetInterval(u64),
    SetPaused(bool),
    SetTouchFree(procsight_core::rules::TouchFreeSet),
    SetTiming(bool),
    /// D7 を通した後の終了
    Exit,
    /// トレイの設定が変わった
    TrayChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKind {
    Ok,
    Warn,
    Danger,
}

/// 1 つしか出さない帯の印
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKey {
    None,
    Paused,
    Late,
    ConfigBroken,
    SuspendedLeft,
    ReplaceMismatch,
    ReplaceGone,
    ReplaceRestore,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BandButton {
    RestartElevated,
    ResumeLeft,
    KeepLeft,
    ReplaceReRegister,
    ReplaceRemove,
    /// E-46 の「オフにする」(外すと同じ)
    ReplaceOff,
    ReplaceRetry,
    ShowTreeResult,
}

#[derive(Clone, Debug)]
pub struct Band {
    pub kind: BandKind,
    pub key: BandKey,
    pub text: String,
    pub buttons: Vec<(&'static str, BandButton, bool)>,
    pub born: f64,
    /// × で閉じられる
    pub closable: bool,
}

/// 取得の状態(詳細のタブ・スタートアップ・サービス・検索)
#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    NotAsked,
    Loading,
    Ready(T),
    Failed(OpError),
}

impl<T> Load<T> {
    pub fn is_not_asked(&self) -> bool {
        matches!(self, Load::NotAsked)
    }
}

/// アイコンの入れ物(D-33)。テクスチャは描くときに作る
pub enum IconSlot {
    Pending,
    Rgba(Vec<u8>),
    Tex(egui::TextureHandle),
    Missing,
}

/// 画面の状態の全部
pub struct State {
    pub env: EnvInfo,
    pub cfg: Config,
    pub snap: Arc<Snapshot>,
    pub diag: Diagnostics,
    pub tab: Tab,
    pub bands: Vec<Band>,
    pub paused: bool,
    pub procs: procs::ProcsView,
    pub perf: perf::PerfView,
    pub startup: startup_view::StartupView,
    pub services: services_view::ServicesView,
    pub settings: settings::SettingsView,
    pub about_open: bool,
    pub locks: locks::LocksView,
    pub dialog: Option<Dialog>,
    pub actions: Vec<Action>,
    pub icons: HashMap<String, IconSlot>,
    pub sigs: HashMap<String, Option<Signature>>,
    pub replace: Option<ReplaceInfo>,
    pub font: Option<LoadedFont>,
    /// 1 フレームの時間(ミリ秒)の直近 200 回(診断)
    pub frame_ms: Vec<f32>,
    pub config_dirty: bool,
    /// ProcSight が一時停止した物(D-37。suspended.toml の中身)
    pub suspended: Vec<SuspendedEntry>,
    pub suspended_dirty: bool,
    /// 起動時に読んだ前回の記録。最初の Snapshot で確かめる(E-12)
    pub suspended_from_last_run: Option<Vec<SuspendedEntry>>,
    pub tree_results: Vec<(Target, Result<(), OpError>)>,
    /// ctx の時刻(秒)
    pub now: f64,
    pub late_dismissed: bool,
    /// 適用済みのテーマと文字の大きさ
    applied_look: Option<(procsight_core::config::Theme, f32)>,
    /// 終了の手順の途中(D7 の「再開してから終了」で結果を待っている)
    pub exit_after_resume: bool,
    /// スレッドの一覧(Job::Threads の結果。D-20)。選んだプロセスの分だけ持つ
    pub threads: HashMap<ProcId, Vec<procsight_core::RawThread>>,
    /// 最後に Job::Threads を送った (識別子, Snapshot の seq)。更新ごとに 1 回だけ取り直す
    pub threads_req: Option<(ProcId, u64)>,
    /// D-40 で戻せなかった Debugger の値(E-45 の「もう一度」で書き戻す)。ログと設定には書かない(INV-7)
    pub replace_saved: Option<String>,
}

impl State {
    pub fn new(env: EnvInfo, cfg: Config) -> State {
        State {
            procs: procs::ProcsView::new(&cfg),
            env,
            cfg,
            snap: Arc::new(Snapshot::default()),
            diag: Diagnostics::default(),
            tab: Tab::Processes,
            bands: Vec::new(),
            paused: false,
            perf: perf::PerfView::default(),
            startup: Default::default(),
            services: Default::default(),
            settings: Default::default(),
            about_open: false,
            locks: Default::default(),
            dialog: None,
            actions: Vec::new(),
            icons: HashMap::new(),
            sigs: HashMap::new(),
            replace: None,
            font: None,
            frame_ms: Vec::new(),
            config_dirty: false,
            suspended: Vec::new(),
            suspended_dirty: false,
            suspended_from_last_run: None,
            tree_results: Vec::new(),
            now: 0.0,
            late_dismissed: false,
            applied_look: None,
            exit_after_resume: false,
            threads: HashMap::new(),
            threads_req: None,
            replace_saved: None,
        }
    }

    pub fn job(&mut self, job: Job) {
        self.actions.push(Action::Job(job));
    }

    pub fn save_config(&mut self) {
        self.config_dirty = true;
    }

    pub fn self_pid(&self) -> u32 {
        self.env.self_pid
    }

    /// 帯を足す。同じ印の物は差し替え、新しい物を上に、最大 3 本
    pub fn band(&mut self, kind: BandKind, text: impl Into<String>) {
        self.push_band(Band { kind, key: BandKey::None, text: text.into(), buttons: Vec::new(), born: self.now, closable: true });
    }

    pub fn push_band(&mut self, mut b: Band) {
        b.born = self.now;
        if b.key != BandKey::None {
            self.bands.retain(|x| x.key != b.key);
        }
        self.bands.insert(0, b);
        // 3 本を超えたら、古い成功の帯 → 古い × で閉じられる帯 → 古い物、の順に消す(一時停止中などの帯を押し出さない)
        while self.bands.len() > 3 {
            let pos = self
                .bands
                .iter()
                .rposition(|b| b.kind == BandKind::Ok)
                .or_else(|| self.bands.iter().rposition(|b| b.closable))
                .unwrap_or(self.bands.len() - 1);
            self.bands.remove(pos);
        }
    }

    pub fn remove_band(&mut self, key: BandKey) {
        self.bands.retain(|b| b.key != key);
    }

    pub fn ok(&mut self, text: impl Into<String>) {
        self.band(BandKind::Ok, text);
    }

    /// 失敗の帯。権限が理由で昇格していなければ「管理者で再起動」を添える(FR-20)
    pub fn fail(&mut self, e: &OpError) {
        let mut b = Band {
            kind: BandKind::Danger,
            key: BandKey::None,
            text: e.message(self.env.elevated),
            buttons: Vec::new(),
            born: self.now,
            closable: true,
        };
        if e.is_access_denied() && !self.env.elevated {
            b.buttons.push(("管理者で再起動", BandButton::RestartElevated, true));
        }
        self.push_band(b);
    }

    /// 新しい Snapshot。seq が変わったときだけ履歴を進める
    pub fn set_snapshot(&mut self, snap: Arc<Snapshot>) {
        if snap.seq == self.snap.seq {
            return;
        }
        self.snap = snap;
        self.perf.push(&self.snap);
        self.procs.on_snapshot(&self.snap);
        // 終わった物は記録から外す(E-13 の他人が止めた物は記録に無い)
        if self.snap.seq > 0 && !self.snap.rows_stale {
            let before = self.suspended.len();
            let rows = &self.snap.rows;
            self.suspended.retain(|e| rows.iter().any(|r| r.id == e.id && r.is_gone == 0));
            if self.suspended.len() != before {
                self.suspended_dirty = true;
            }
        }
        if let Some(last) = self.suspended_from_last_run.take() {
            self.check_left_suspended(last);
        }
    }

    /// E-12: 前回の記録のうち、同じ識別子でまだ一時停止中の物
    fn check_left_suspended(&mut self, last: Vec<SuspendedEntry>) {
        if self.snap.seq == 0 {
            self.suspended_from_last_run = Some(last);
            return;
        }
        let still: Vec<SuspendedEntry> = last
            .into_iter()
            .filter(|e| self.snap.find(e.id).is_some_and(|r| r.state == ProcState::Suspended && r.is_gone == 0))
            .collect();
        if still.is_empty() {
            self.suspended_dirty = true;
            return;
        }
        self.push_band(Band {
            kind: BandKind::Warn,
            key: BandKey::SuspendedLeft,
            text: format!("前回一時停止したプロセスが {} 個残っています", still.len()),
            buttons: vec![("再開する", BandButton::ResumeLeft, true), ("そのままにする", BandButton::KeepLeft, true)],
            born: self.now,
            closable: false,
        });
        self.suspended = still;
        self.suspended_dirty = true;
    }

    pub fn set_diagnostics(&mut self, d: Diagnostics) {
        if d.late_in_row >= 3 && !self.late_dismissed && !self.bands.iter().any(|b| b.key == BandKey::Late) {
            self.push_band(Band {
                kind: BandKind::Warn,
                key: BandKey::Late,
                text: "取得が間に合っていません。更新間隔を長くしてください".into(),
                buttons: Vec::new(),
                born: self.now,
                closable: true,
            });
        }
        if d.late_in_row == 0 {
            self.late_dismissed = false;
        }
        self.diag = d;
    }

    /// 起動時に設定を読めなかった(E-41)
    pub fn config_broken(&mut self, name: &str) {
        self.push_band(Band {
            kind: BandKind::Warn,
            key: BandKey::ConfigBroken,
            text: format!(
                "設定のファイルを読めなかったため、既定の設定で起動しました。元のファイルは {name} に残しました"
            ),
            buttons: Vec::new(),
            born: self.now,
            closable: true,
        });
    }

    pub fn target_of(&self, id: ProcId) -> Option<Target> {
        self.snap.find(id).map(|r| Target { id, name: r.name.clone() })
    }

    /// 裏の処理の結果を受ける
    pub fn apply(&mut self, r: JobResult) {
        match r {
            JobResult::Terminated { results, tree } => self.on_terminated(results, tree),
            JobResult::Op { job, result } => self.on_op(job, result),
            JobResult::Affinity { target, result } => dialogs::on_affinity(self, target, result),
            JobResult::General { id, info } => details::on_general(self, id, info),
            JobResult::Modules { id, result } => details::on_modules(self, id, result),
            JobResult::Threads { id, result } => {
                self.threads.retain(|k, _| *k == id);
                match result {
                    Ok(v) => {
                        self.threads.insert(id, v);
                    }
                    Err(_) => {
                        self.threads.remove(&id);
                    }
                }
            }
            JobResult::Handles { id, result } => details::on_handles(self, id, result),
            JobResult::Signature { path, result } => {
                self.sigs.insert(path, Some(result));
                self.procs.invalidate();
            }
            JobResult::Locking { paths, result } => locks::on_result(self, paths, result),
            JobResult::Startup { result } => startup_view::on_loaded(self, result),
            JobResult::StartupSet { name, enabled, result } => startup_view::on_set(self, name, enabled, result),
            JobResult::Services { result } => services_view::on_loaded(self, result),
            JobResult::ServiceDependents { name, result } => dialogs::on_dependents(self, name, result),
            JobResult::ServiceDone { display_name, action, result } => {
                services_view::on_done(self, display_name, action, result)
            }
            JobResult::StartTypeSet { display_name, start_type, result } => {
                services_view::on_start_type(self, display_name, start_type, result)
            }
            JobResult::Icon { path, rgba } => {
                let slot = match rgba {
                    Some(v) if v.len() == 16 * 16 * 4 => IconSlot::Rgba(v),
                    _ => IconSlot::Missing,
                };
                self.icons.insert(path, slot);
            }
            JobResult::RestartElevated { result } => match result {
                Ok(()) => self.actions.push(Action::Exit),
                Err(e) => self.fail(&e),
            },
            JobResult::Replace { info } => settings::on_replace_info(self, info),
            JobResult::ReplaceChanged { enabled, previous, result } => {
                settings::on_replace_changed(self, enabled, previous, result)
            }
            JobResult::ReplaceRestored { result } => match result {
                Ok(()) => {
                    self.replace_saved = None;
                    self.remove_band(BandKey::ReplaceRestore);
                    self.ok("Ctrl+Shift+Esc の置き換えを戻しました");
                    crate::log::info("replace_restore", "result=ok");
                    self.job(Job::ReplaceStatus);
                }
                Err(e) => {
                    crate::log::warn("replace_restore", "result=failed");
                    self.fail(&e);
                }
            },
            JobResult::ReplaceRestoreFailed { saved } => {
                self.replace_saved = Some(saved);
                self.push_band(Band {
                    kind: BandKind::Danger,
                    key: BandKey::ReplaceRestore,
                    text: "Ctrl+Shift+Esc の置き換えを戻せませんでした".into(),
                    buttons: vec![("もう一度", BandButton::ReplaceRetry, true)],
                    born: self.now,
                    closable: false,
                });
                crate::log::warn("replace_restore", "result=failed");
                self.job(Job::ReplaceStatus);
            }
            JobResult::PickedFiles { paths } => locks::on_picked(self, paths),
        }
    }

    fn on_terminated(&mut self, results: Vec<(Target, Result<(), OpError>)>, tree: bool) {
        for (t, r) in &results {
            let res = if r.is_ok() { "ok" } else { "failed" };
            crate::log::info("terminate", &format!("pid={} name={} result={res}", t.id.pid, crate::log::q(&t.name)));
        }
        let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
        let failed: Vec<&(Target, Result<(), OpError>)> = results.iter().filter(|(_, r)| r.is_err()).collect();
        if tree {
            if failed.is_empty() {
                self.ok(format!("{ok} 個のプロセスを終了しました"));
            } else {
                self.tree_results = results.clone();
                self.push_band(Band {
                    kind: BandKind::Danger,
                    key: BandKey::None,
                    text: format!("{ok} 個を終了しました。{} 個は終了できませんでした", failed.len()),
                    buttons: vec![("結果を見る", BandButton::ShowTreeResult, true)],
                    born: self.now,
                    closable: true,
                });
            }
            return;
        }
        match (results.len(), failed.len()) {
            (1, 0) => {
                let t = &results[0].0;
                self.ok(format!("{}(PID {})を終了しました", t.name, t.id.pid));
            }
            (_, 0) => self.ok(format!("{ok} 個のプロセスを終了しました")),
            _ => {
                let errs: Vec<(Target, OpError)> =
                    failed.iter().filter_map(|(t, r)| r.clone().err().map(|e| (t.clone(), e))).collect();
                if ok > 0 {
                    self.ok(format!("{ok} 個のプロセスを終了しました"));
                }
                for (t, e) in errs.into_iter().take(2) {
                    let mut msg = e.message(self.env.elevated);
                    if results.len() > 1 {
                        msg = format!("{}(PID {}): {msg}", t.name, t.id.pid);
                    }
                    let mut b = Band { kind: BandKind::Danger, key: BandKey::None, text: msg, buttons: Vec::new(), born: self.now, closable: true };
                    if e.is_access_denied() && !self.env.elevated {
                        b.buttons.push(("管理者で再起動", BandButton::RestartElevated, true));
                    }
                    self.push_band(b);
                }
            }
        }
    }

    fn on_op(&mut self, job: Job, result: Result<(), OpError>) {
        let name_pid = |t: &Target| format!("pid={} name={}", t.id.pid, crate::log::q(&t.name));
        let (event, target) = match &job {
            Job::SetPriority { target, .. } => ("priority", Some(target)),
            Job::SetAffinity { target, .. } => ("affinity", Some(target)),
            Job::SetEfficiency { target, .. } => ("efficiency", Some(target)),
            Job::Suspend { target } => ("suspend", Some(target)),
            Job::Resume { target } => ("resume", Some(target)),
            Job::ResumeMany { .. } => ("resume_many", None),
            Job::BringToFront { target } => ("front", Some(target)),
            Job::OpenTaskManager => ("open_taskmgr", None),
            _ => ("op", None),
        };
        let res = if result.is_ok() { "ok" } else { "failed" };
        crate::log::info(event, &format!("{}{}result={res}", target.map(name_pid).unwrap_or_default(), if target.is_some() { " " } else { "" }));
        if let Err(e) = result {
            if matches!(job, Job::ResumeMany { .. }) && self.exit_after_resume {
                self.actions.push(Action::Exit);
            }
            self.fail(&e);
            return;
        }
        match job {
            Job::SetPriority { target, priority } => {
                self.ok(format!("{} の優先度を『{}』にしました", target.name, priority.label()))
            }
            Job::SetAffinity { target, mask } => {
                self.ok(format!("{} の CPU の割り当てを変えました({} 個)", target.name, mask.count_ones()))
            }
            Job::SetEfficiency { target, on } => {
                self.ok(format!("{} の効率モードを{}", target.name, if on { "入れました" } else { "切りました" }));
                details::forget_general(self, target.id);
            }
            Job::Suspend { target } => {
                self.ok(format!("{} を一時停止しました", target.name));
                if !self.suspended.iter().any(|e| e.id == target.id) {
                    let at = crate::files::iso_time(crate::files::now_filetime(), self.env.utc_offset_minutes, false);
                    self.suspended.push(SuspendedEntry { id: target.id, name: target.name, suspended_at: at });
                    self.suspended_dirty = true;
                }
            }
            Job::Resume { target } => {
                self.ok(format!("{} を再開しました", target.name));
                self.suspended.retain(|e| e.id != target.id);
                self.suspended_dirty = true;
            }
            Job::ResumeMany { targets } => {
                self.ok(format!("{} 個のプロセスを再開しました", targets.len()));
                self.suspended.retain(|e| !targets.iter().any(|t| t.id == e.id));
                self.suspended_dirty = true;
                if self.exit_after_resume {
                    self.actions.push(Action::Exit);
                }
            }
            _ => {}
        }
    }

    /// 終了の前に、ProcSight が止めてまだ止まっている物(D7・FR-51)
    pub fn still_suspended_by_me(&self) -> Vec<Target> {
        self.suspended
            .iter()
            .filter(|e| self.snap.find(e.id).is_some_and(|r| r.state == ProcState::Suspended && r.is_gone == 0))
            .map(|e| Target { id: e.id, name: e.name.clone() })
            .collect()
    }

    /// 窓の × やトレイの「終了」。一時停止中が残っていれば D7 を出す
    pub fn request_exit(&mut self) {
        let left = self.still_suspended_by_me();
        if left.is_empty() {
            self.actions.push(Action::Exit);
        } else {
            self.dialog = Some(Dialog::ExitSuspended { targets: left, opened: false });
        }
    }

    fn band_button(&mut self, b: BandButton) {
        match b {
            BandButton::RestartElevated => self.dialog = Some(Dialog::RestartElevated { opened: false }),
            BandButton::ResumeLeft => {
                let targets: Vec<Target> = self.suspended.iter().map(|e| Target { id: e.id, name: e.name.clone() }).collect();
                self.job(Job::ResumeMany { targets });
                self.remove_band(BandKey::SuspendedLeft);
            }
            BandButton::KeepLeft => {
                // 記録は残す(終了時の確認に出す)
                self.remove_band(BandKey::SuspendedLeft);
            }
            BandButton::ReplaceReRegister => {
                self.job(Job::ReplaceEnable);
                self.remove_band(BandKey::ReplaceMismatch);
                self.remove_band(BandKey::ReplaceGone);
            }
            // E-45: 外す前の値をそのまま書き戻す(置き換えを新しくオンにはしない)
            BandButton::ReplaceRetry => {
                if let Some(value) = self.replace_saved.clone() {
                    self.job(Job::ReplaceRestore { value });
                }
            }
            BandButton::ReplaceRemove | BandButton::ReplaceOff => {
                self.job(Job::ReplaceDisable { previous: self.cfg.taskmgr_replace.previous_debugger.clone() });
                self.remove_band(BandKey::ReplaceMismatch);
                self.remove_band(BandKey::ReplaceGone);
            }
            BandButton::ShowTreeResult => {
                self.dialog = Some(Dialog::TreeResult { results: self.tree_results.clone() });
            }
        }
    }

    /// 1 フレーム分を描く
    pub fn ui(&mut self, ui: &mut Ui) {
        let t0 = std::time::Instant::now();
        self.now = ui.input(|i| i.time);
        let look = (self.cfg.theme, self.cfg.font_scale);
        if self.applied_look != Some(look) {
            theme::apply(ui.ctx(), look.0, look.1);
            self.applied_look = Some(look);
        }
        // 5 秒たった成功の帯を消す
        let now = self.now;
        self.bands.retain(|b| b.kind != BandKind::Ok || now - b.born < 5.0);
        if self.bands.iter().any(|b| b.kind == BandKind::Ok) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.keyboard(ui);
        let pal = theme::palette(ui);

        egui::Panel::top("top").show(ui, |ui| self.top(ui, &pal));
        if !self.bands.is_empty() {
            egui::Panel::top("bands").show(ui, |ui| self.bands_ui(ui, &pal));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_line(ui, &pal));
        egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
            Tab::Processes => procs::ui(self, ui),
            Tab::Performance => perf::ui(self, ui),
            Tab::Startup => startup_view::ui(self, ui),
            Tab::Services => services_view::ui(self, ui),
        });
        settings::window(self, ui);
        crate::about::window(self, ui);
        locks::window(self, ui);
        dialogs::show(self, ui);

        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        if self.frame_ms.len() >= 200 {
            self.frame_ms.remove(0);
        }
        self.frame_ms.push(ms);
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        if self.dialog.is_some() {
            return;
        }
        let tabs = [Key::Num1, Key::Num2, Key::Num3, Key::Num4];
        for (k, t) in tabs.into_iter().zip(Tab::ALL) {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, k)) {
                self.tab = t;
            }
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            match self.tab {
                Tab::Processes => self.procs.focus_filter = true,
                Tab::Services => self.services.focus_filter = true,
                _ => {}
            }
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
            match self.tab {
                Tab::Startup => self.job_load_startup(),
                Tab::Services => self.job_load_services(),
                _ => {}
            }
        }
    }

    pub fn job_load_startup(&mut self) {
        self.startup.items = Load::Loading;
        self.job(Job::LoadStartup);
    }

    pub fn job_load_services(&mut self) {
        self.services.list = Load::Loading;
        self.job(Job::LoadServices);
    }

    fn top(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("ファイル", |ui| {
                    if !self.env.elevated && ui.button("管理者で再起動").clicked() {
                        self.dialog = Some(Dialog::RestartElevated { opened: false });
                    }
                    if ui.button("終了").clicked() {
                        self.request_exit();
                    }
                });
                ui.menu_button("ツール", |ui| {
                    if ui.button("ファイルを掴んでいるプロセスを探す…").clicked() {
                        self.locks.open = true;
                    }
                });
                ui.menu_button("設定", |ui| {
                    if ui.button("設定…").clicked() {
                        settings::open(self);
                    }
                });
                ui.menu_button("ヘルプ", |ui| {
                    if ui.button("このソフトについて…").clicked() {
                        self.about_open = true;
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.env.elevated {
                    // 強調の色の印。Frame は右寄せの中で右へはみ出すので、押せないボタンの形で出す
                    let badge = egui::Button::new(RichText::new("管理者").color(Color32::WHITE).strong())
                        .fill(pal.accent)
                        .sense(egui::Sense::hover());
                    ui.add(badge).on_hover_text("ProcSight は管理者の権限で動いています");
                } else if ui.button("管理者で再起動").clicked() {
                    self.dialog = Some(Dialog::RestartElevated { opened: false });
                }
            });
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            for t in Tab::ALL {
                let sel = self.tab == t;
                let text = if sel { RichText::new(t.label()).strong() } else { RichText::new(t.label()) };
                if ui.selectable_label(sel, text).clicked() {
                    self.tab = t;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.paused { "再開" } else { "一時停止" };
                if ui.button(label).clicked() {
                    self.set_paused(!self.paused);
                }
                let mut ms = self.cfg.interval_ms;
                let combo = egui::ComboBox::from_id_salt("interval")
                    .selected_text(interval_label(ms))
                    .show_ui(ui, |ui| {
                        for v in INTERVALS_MS {
                            ui.selectable_value(&mut ms, v, interval_label(v));
                        }
                    })
                    .response;
                if ms != self.cfg.interval_ms {
                    self.set_interval(ms);
                }
                let l = ui.label("更新:");
                combo.labelled_by(l.id);
            });
        });
        ui.add_space(2.0);
    }

    pub fn set_interval(&mut self, ms: u64) {
        self.cfg.interval_ms = ms;
        self.perf.clear();
        self.actions.push(Action::SetInterval(ms));
        self.save_config();
    }

    pub fn set_paused(&mut self, p: bool) {
        self.paused = p;
        self.actions.push(Action::SetPaused(p));
        if p {
            self.push_band(Band {
                kind: BandKind::Warn,
                key: BandKey::Paused,
                text: "更新を一時停止しています".into(),
                buttons: Vec::new(),
                born: self.now,
                closable: false,
            });
        } else {
            self.remove_band(BandKey::Paused);
        }
    }

    fn bands_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let mut close = None;
        let mut press = None;
        for (i, b) in self.bands.iter().enumerate() {
            let (fill, fg) = match b.kind {
                BandKind::Ok => (pal.band_ok, pal.accent),
                BandKind::Warn => (pal.band_warn, pal.warn),
                BandKind::Danger => (pal.band_danger, pal.danger),
            };
            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(8, 4)).corner_radius(3.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&b.text).color(fg));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if b.closable && ui.small_button("×").on_hover_text("閉じる").clicked() {
                            close = Some(i);
                        }
                        for (label, btn, enabled) in b.buttons.iter().rev() {
                            if ui.add_enabled(*enabled, egui::Button::new(*label)).clicked() {
                                press = Some((i, btn.clone()));
                            }
                        }
                    });
                });
            });
            ui.add_space(2.0);
        }
        if let Some(i) = close {
            if self.bands[i].key == BandKey::Late {
                self.late_dismissed = true;
            }
            self.bands.remove(i);
        }
        if let Some((i, b)) = press {
            if !matches!(b, BandButton::ResumeLeft | BandButton::KeepLeft) && i < self.bands.len() && self.bands[i].key == BandKey::None {
                self.bands.remove(i);
            }
            self.band_button(b);
        }
    }

    fn status_line(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            let t = &self.snap.totals;
            let p = &self.snap.perf;
            let mem = p.mem.as_ref().and_then(|m| m.used_pct()).map_or_else(|| format::DASH.to_string(), |v| format!("{v:.0}%"));
            ui.label(format!("プロセス {}", format::thousands(t.processes as u64)));
            ui.add_space(12.0);
            ui.label(format!("スレッド {}", format::thousands(t.threads as u64)));
            ui.add_space(12.0);
            ui.label(format!("CPU {}", format::pct(p.cpu_total)));
            ui.add_space(12.0);
            ui.label(format!("メモリ {mem}"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.env.elevated {
                    ui.label(RichText::new("管理者").color(pal.accent));
                } else {
                    ui.label(RichText::new("標準の権限").color(pal.weak));
                }
            });
        });
    }
}

pub fn interval_label(ms: u64) -> String {
    if ms.is_multiple_of(1000) { format!("{} 秒", ms / 1000) } else { format!("{:.1} 秒", ms as f64 / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(interval_label(500), "0.5 秒");
        assert_eq!(interval_label(4000), "4 秒");
    }

    #[test]
    fn bands_keep_three_newest_and_replace_same_key() {
        let mut st = crate::demo::state();
        st.bands.clear();
        for i in 0..5 {
            st.band(BandKind::Ok, format!("{i}"));
        }
        assert_eq!(st.bands.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), vec!["4", "3", "2"]);
        st.set_paused(true);
        st.set_paused(true);
        assert_eq!(st.bands.iter().filter(|b| b.key == BandKey::Paused).count(), 1);
        st.set_paused(false);
        assert!(st.bands.iter().all(|b| b.key != BandKey::Paused));
    }

    #[test]
    fn access_denied_offers_restart_only_when_not_elevated() {
        let mut st = crate::demo::state();
        st.bands.clear();
        st.fail(&OpError::Win32(5));
        assert_eq!(st.bands[0].buttons.len(), 1);
        assert!(st.bands[0].text.starts_with("権限が足りません"));
        st.env.elevated = true;
        st.fail(&OpError::Win32(5));
        assert!(st.bands[0].buttons.is_empty());
        assert!(st.bands[0].text.contains("保護されている"));
    }

    #[test]
    fn suspend_result_is_recorded_and_resume_removes_it() {
        let mut st = crate::demo::state();
        let r = st.snap.rows.iter().find(|r| r.name == "notepad.exe").unwrap();
        let t = Target { id: r.id, name: r.name.clone() };
        st.apply(JobResult::Op { job: Job::Suspend { target: t.clone() }, result: Ok(()) });
        assert!(st.suspended.iter().any(|e| e.id == t.id) && st.suspended_dirty);
        st.apply(JobResult::Op { job: Job::Resume { target: t.clone() }, result: Ok(()) });
        assert!(!st.suspended.iter().any(|e| e.id == t.id));
    }

    #[test]
    fn leftover_suspended_band_only_for_still_suspended() {
        // E-12: 同じ識別子でまだ一時停止中の物だけを数える
        let mut st = crate::demo::state();
        let susp = st.snap.rows.iter().find(|r| r.state == ProcState::Suspended).unwrap().id;
        let gone = ProcId { pid: 99_999, create_time: 1 };
        let mk = |id| SuspendedEntry { id, name: "x.exe".into(), suspended_at: String::new() };
        st.suspended_from_last_run = Some(vec![mk(susp), mk(gone)]);
        let mut next = (*st.snap).clone();
        next.seq += 1;
        st.set_snapshot(Arc::new(next));
        let b = st.bands.iter().find(|b| b.key == BandKey::SuspendedLeft).expect("帯");
        assert!(b.text.contains("1 個"));
    }
}
