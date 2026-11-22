//! 起動の手順(§9.10・D-18・D-24・D-37・E-34・E-41)と、Backend を持つ eframe::App(殻)。
//! 殻は Snapshot と JobResult を State に渡し、State が積んだ Action を Backend に渡すだけ。画面の中身は app.rs から先。
//! procsight-win の関数を直接呼ぶのは INTERFACES §3.2 の例外(起動時・クリップボード・トレイ)だけ(INV-9)。

use crate::app::{Action, State};
use crate::tray::{Tray, TrayEvent};
use crate::{files, fonts, log};
use eframe::egui;
use egui::ViewportCommand;
use procsight_core::config::CloseAction;
use procsight_core::taskmgr::{self, LaunchArgs};
use procsight_win::jobs::Job;
use procsight_win::single_instance::{self, Acquire, Instance};
use procsight_win::{Backend, BackendConfig, clipboard, elevation};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

pub struct Shell {
    backend: Option<Backend>,
    st: State,
    _instance: Instance,
    activate: Receiver<()>,
    tray: Option<Tray>,
    hidden: bool,
    minimized: bool,
    allow_close: bool,
    config_dir: Option<PathBuf>,
    local_dir: Option<PathBuf>,
    last_seq: u64,
    save_at: Option<Instant>,
    timing_log_at: Instant,
}

impl Shell {
    fn new(cc: &eframe::CreationContext<'_>, mut st: State, instance: Instance, after_restart: bool) -> Shell {
        st.font = fonts::install(&cc.egui_ctx, &st.cfg.fonts);
        match &st.font {
            Some(f) => log::info("font", &format!("file={} index={}", log::q(log::file_name(&f.path)), f.index)),
            None => log::warn("font", "result=none"),
        }
        let ctx = cc.egui_ctx.clone();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let backend = Backend::start(BackendConfig { interval_ms: st.cfg.interval_ms, touch_free: st.cfg.touch_free_set() }, repaint);
        let (tx, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        instance.listen(Box::new(move || {
            let _ = tx.send(());
            ctx.request_repaint();
        }));
        // FR-56: 起動時に置き換えの値を読む
        backend.submit(Job::ReplaceStatus);
        // 設定「起動時に管理者へ切り替える」。この起動では D8 を出さない
        if st.cfg.elevate_on_start && !st.env.elevated && !after_restart {
            backend.submit(Job::RestartElevated);
        }
        let tray = if st.cfg.tray_enabled { Tray::new(&cc.egui_ctx) } else { None };
        Shell {
            backend: Some(backend),
            st,
            _instance: instance,
            activate: rx,
            tray,
            hidden: false,
            minimized: false,
            allow_close: false,
            config_dir: files::config_dir(),
            local_dir: files::local_dir(),
            last_seq: u64::MAX,
            save_at: None,
            timing_log_at: Instant::now(),
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn run_actions(&mut self, ctx: &egui::Context) {
        let Some(b) = self.backend.as_ref() else { return };
        for a in std::mem::take(&mut self.st.actions) {
            match a {
                Action::Job(j) => b.submit(j),
                Action::SetInterval(ms) => b.set_interval(ms),
                Action::SetPaused(p) => b.set_paused(p),
                Action::SetTouchFree(t) => b.set_touch_free(t),
                Action::SetTiming(on) => b.set_timing(on),
                Action::Exit => {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Action::TrayChanged => {
                    self.tray = if self.st.cfg.tray_enabled { self.tray.take().or_else(|| Tray::new(ctx)) } else { None };
                }
            }
        }
    }

    fn save(&mut self, force: bool) {
        if self.st.config_dirty {
            let at = *self.save_at.get_or_insert_with(|| Instant::now() + Duration::from_millis(400));
            if force || Instant::now() >= at {
                if let Some(d) = &self.config_dir
                    && files::save_config(d, &self.st.cfg).is_err()
                {
                    log::warn("config_save", "result=failed");
                }
                self.st.config_dirty = false;
                self.save_at = None;
            }
        }
        if self.st.suspended_dirty {
            if let Some(d) = &self.local_dir
                && files::save_suspended(d, &self.st.suspended).is_err()
            {
                log::warn("suspended_save", "result=failed");
            }
            self.st.suspended_dirty = false;
        }
    }

    fn log_timing(&mut self) {
        if !self.st.diag.timing || self.timing_log_at.elapsed() < Duration::from_secs(10) {
            return;
        }
        self.timing_log_at = Instant::now();
        let f = |v: &[f32]| crate::settings::stats(v).map_or_else(|| "-".into(), |(m, p, x)| format!("{m:.1}/{p:.1}/{x:.1}"));
        log::info("timing", &format!("collect_ms={} frame_ms={}", f(&self.st.diag.collect_ms), f(&self.st.frame_ms)));
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(b) = self.backend.as_ref() else { return };
        while let Some(r) = b.try_recv() {
            self.st.apply(r);
        }
        let snap = b.snapshot();
        if snap.seq != self.last_seq {
            self.last_seq = snap.seq;
            if let Some(t) = self.tray.as_mut() {
                t.set_cpu(snap.perf.cpu_total);
            }
            self.st.set_snapshot(snap);
            self.st.set_diagnostics(b.diagnostics());
        }
        if self.activate.try_recv().is_ok() {
            self.show_window(ctx);
        }
        let events = self.tray.as_ref().map(|t| t.poll()).unwrap_or_default();
        for e in events {
            self.show_window(ctx);
            if e == TrayEvent::Exit {
                self.st.request_exit();
            }
        }
        // D-36: 最小化・隠している間はプロセスごとの値を作らない
        let (minimized, close, outer, inner, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.minimized.unwrap_or(false), v.close_requested(), v.outer_rect, v.inner_rect, v.maximized.unwrap_or(false))
        });
        let m = minimized || self.hidden;
        if m != self.minimized {
            self.minimized = m;
            if let Some(b) = self.backend.as_ref() {
                b.set_minimized(m);
            }
        }
        if !minimized && !self.hidden {
            let w = &mut self.st.cfg.window;
            w.maximized = maximized;
            // Windows は最小化中の位置を -32000 で返す。その値は覚えない
            if !maximized
                && let (Some(o), Some(i)) = (outer, inner)
                && plausible_position(o.min.x, o.min.y)
            {
                w.x = o.min.x.round() as i32;
                w.y = o.min.y.round() as i32;
                w.width = i.width().round().max(400.0) as u32;
                w.height = i.height().round().max(300.0) as u32;
            }
        }
        // 隠している間の入力は最後に描いたフレームの物なので、閉じる要求は見ない
        if close && !self.allow_close && !self.hidden {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if self.st.cfg.effective_close_action() == CloseAction::Tray && self.tray.is_some() {
                self.hidden = true;
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            } else {
                self.st.request_exit();
            }
        }
        self.run_actions(ctx);
        self.save(false);
        self.log_timing();
        if self.st.config_dirty {
            ctx.request_repaint_after(Duration::from_millis(450));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.st.ui(ui);
        let ctx = ui.ctx().clone();
        // クリップボードは Win32 で行う(INTERFACES §3.5)
        ctx.output_mut(|o| {
            o.commands.retain(|c| match c {
                egui::OutputCommand::CopyText(t) => {
                    clipboard::set_text(t);
                    false
                }
                _ => true,
            })
        });
        self.run_actions(&ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // clipboard_shortcuts を切ってあるので Ctrl+X/C/V は Key のまま届く。egui-winit と同じ形に直す
        clipboard_keys(&mut raw.events, clipboard::get_text);
    }

    fn on_exit(&mut self) {
        self.st.config_dirty = true;
        self.save(true);
        if let Some(b) = self.backend.take() {
            let (touch, total) = b.shutdown();
            log::info("gate_summary", &format!("touch_free_opens={touch} total_opens={total}"));
        }
        log::info("exit", "");
    }
}

/// 窓の位置として使ってよい値か。最小化中の -32000 と、どの画面にもありそうにない値を捨てる(§9.3)。
/// ponytail: 画面の大きさは見ていない。外したモニターの位置は残る(報告済み)
pub fn plausible_position(x: f32, y: f32) -> bool {
    (-16_000.0..=16_000.0).contains(&x) && (-16_000.0..=16_000.0).contains(&y)
}

/// Ctrl+X / Ctrl+C / Ctrl+V の Key を Cut / Copy / Paste に直す(egui-winit の clipboard_shortcuts と同じ形)。
/// 貼り付けの中身は `get`(Win32 のクリップボード)から取る
pub fn clipboard_keys(events: &mut Vec<egui::Event>, get: impl Fn() -> Option<String>) {
    let mut out = Vec::with_capacity(events.len());
    for e in events.drain(..) {
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = &e
            && modifiers.command
            && !modifiers.alt
            && !modifiers.shift
        {
            match key {
                egui::Key::X => {
                    out.push(egui::Event::Cut);
                    continue;
                }
                egui::Key::C => {
                    out.push(egui::Event::Copy);
                    continue;
                }
                egui::Key::V => {
                    if let Some(t) = get().map(|t| t.replace("\r\n", "\n")).filter(|t| !t.is_empty()) {
                        out.push(egui::Event::Paste(t));
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(e);
    }
    *events = out;
}


/// main から呼ぶ。終了コード: 0 = 正常、2 = 別の ProcSight を前面に出した、3 = 窓を作れなかった(§9.10)
pub fn run() -> ExitCode {
    procsight_win::harden_dll_search();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let launch = taskmgr::parse_args(&args);
    let env = procsight_win::env_info();
    let local = files::local_dir();
    if let Some(d) = &local {
        log::init(d.join("logs"), env.utc_offset_minutes);
    }
    log::info("start", &format!("version={} elevated={}", env!("CARGO_PKG_VERSION"), env.elevated));
    let after_restart = match &launch {
        LaunchArgs::AfterRestart(pid) => {
            elevation::wait_pid_exit(*pid, 5000);
            true
        }
        // 中身はパスやユーザー名を含みうるので、数だけ書く(INV-7)
        LaunchArgs::Unknown(v) => {
            log::warn("unknown_args", &format!("count={}", v.len()));
            false
        }
        LaunchArgs::FromTaskmgr => {
            log::info("from_taskmgr", "");
            false
        }
        LaunchArgs::Normal => false,
    };
    let instance = match single_instance::acquire() {
        Acquire::First(i) => i,
        Acquire::ActivatedOther => {
            log::info("single_instance", "result=activated_other");
            return ExitCode::from(2);
        }
        Acquire::DeniedByElevated => {
            // E-34
            log::warn("single_instance", "result=denied_by_elevated");
            procsight_win::util::message_box("管理者で動いている ProcSight があります。そちらを使ってください");
            return ExitCode::from(2);
        }
    };
    let loaded = match files::config_dir() {
        Some(d) => files::load_config(&d, env.utc_offset_minutes),
        None => files::Loaded { config: Default::default(), warnings: Vec::new(), broken: None },
    };
    match &loaded.broken {
        // 壊れた理由の文には設定の値が入りうるので書かない
        Some(name) => log::warn("config_broken", &format!("moved_to={}", log::q(name))),
        None => {
            for w in &loaded.warnings {
                log::warn("config", &format!("message={}", log::q(w)));
            }
        }
    }
    let mut st = State::new(env, loaded.config);
    if let Some(name) = &loaded.broken {
        st.config_broken(name);
    }
    if let Some(d) = &local {
        let last = files::load_suspended(d);
        if !last.is_empty() {
            st.suspended_from_last_run = Some(last);
        }
    }
    let mut w = st.cfg.window.clone();
    if !plausible_position(w.x as f32, w.y as f32) {
        let d = procsight_core::config::WindowPlacement::default();
        (w.x, w.y) = (d.x, d.y);
    }
    let icon = egui::IconData { rgba: crate::icon_art::app_icon_rgba(32), width: 32, height: 32 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ProcSight")
            .with_inner_size([w.width as f32, w.height as f32])
            .with_position([w.x as f32, w.y as f32])
            .with_maximized(w.maximized)
            .with_min_inner_size([640.0, 400.0])
            .with_icon(Arc::new(icon))
            .with_drag_and_drop(true),
        // Ctrl+X/C/V を Key のまま受け、raw_input_hook で Win32 のクリップボードにつなぐ(INTERFACES §3.5)
        clipboard_shortcuts: false,
        ..Default::default()
    };
    let result = eframe::run_native("ProcSight", options, Box::new(move |cc| Ok(Box::new(Shell::new(cc, st, instance, after_restart)))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            // 窓を作れなかった(§9.10 の終了コード 3)
            log::warn("window", "result=failed");
            procsight_win::util::message_box("ProcSight の窓を作れませんでした。グラフィックスのドライバーを確かめてください");
            ExitCode::from(3)
        }
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    fn key(k: egui::Key, ctrl: bool) -> egui::Event {
        let modifiers = if ctrl { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn ctrl_v_becomes_paste_from_clipboard() {
        let mut ev = vec![key(egui::Key::V, true), key(egui::Key::C, true), key(egui::Key::A, true), key(egui::Key::V, false)];
        clipboard_keys(&mut ev, || Some("a\r\nb".into()));
        assert_eq!(ev[0], egui::Event::Paste("a\nb".into()));
        assert_eq!(ev[1], egui::Event::Copy);
        assert!(matches!(ev[2], egui::Event::Key { key: egui::Key::A, .. }));
        assert!(matches!(ev[3], egui::Event::Key { key: egui::Key::V, .. }), "Ctrl なしの V はそのまま");
        let mut empty = vec![key(egui::Key::V, true)];
        clipboard_keys(&mut empty, || None);
        assert!(empty.is_empty());
    }

    #[test]
    fn minimized_position_is_not_kept() {
        assert!(plausible_position(100.0, 100.0));
        assert!(plausible_position(-1900.0, 0.0), "左のモニター");
        assert!(!plausible_position(-32000.0, -32000.0));
    }
}
