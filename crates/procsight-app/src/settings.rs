//! S5 設定と S6 診断(画面設計書 §7.1〜§7.4)。変えたらすぐ反映して config.toml に書く(保存のボタンは無い。FR-48)。
//! Ctrl+Shift+Esc の置き換え(FR-53・FR-56)は状態を Job で読み、オンは D9 を通す。
//! 左はページの並び(選んでいる物に強調の色の縦の棒)、右は見出しの線で区切った項目。診断は S6。

use eframe::egui;
use crate::app::{Action, Band, BandButton, BandKey, BandKind, State};
use crate::dialogs::Dialog;
use crate::services_view::segmented;
use crate::theme::{self, Palette, size, space};
use crate::widgets;
use egui::{Align2, CornerRadius, FontId, Rect, RichText, Sense, Ui, Vec2};
use procsight_core::config::{
    Accent, CloseAction, CpuScale, Density, DoubleClick, GRAPH_POINTS_CHOICES, HeatFull, HeatStart, INTERVALS_MS, SpeedUnit, StartMode, StartTab, Theme, TrayValue, TreeStart, View,
};
use procsight_core::format;
use procsight_core::rules::{self, MAX_TOUCH_FREE};
use procsight_core::taskmgr::{self, ReplaceStatus};
use procsight_win::jobs::{Job, OpError, ReplaceInfo};

pub const REPLACE_TEXT: &str = "オンにすると、Windows のタスク マネージャーの代わりに ProcSight が開きます。Windows のタスク マネージャーは、プロセスのメニューの『Windows のタスク マネージャーを開く』から開けます。ProcSight が起動できなくなったときは、配布物の restore-taskmgr.reg をダブルクリックすると元に戻ります。";

pub const TOUCH_FREE_TEXT: &str = "ここに登録した exe のプロセスには、ProcSight からハンドルを開きません。一覧には出しますが、詳細の取得と操作は行いません。ゲームのアンチチートに怪しまれないようにするための設定です。ただし、これで安全だと保証するものではありません。exe 名(例: game.exe)を 1 行に 1 つ入れてください。";

pub const SLOW_TEXT: &str = "ゲームのアンチチートとは関係なく、ProcSight 自身を軽くするための設定です。触れないプロセスが終わってから 30 秒で元の間隔に戻ります";

pub const ALWAYS_ON_TOP_TEXT: &str = "ほかの窓の上に出したままにします。ミニ表示(Ctrl+M)と合わせると、ゲームの横や別の画面に小さく置けます";

pub const HEAT_MAP_TEXT: &str = "CPU やメモリの列の背景を、使っている量に合わせて塗ります";

pub const CPU_SCALE_TEXT: &str = "プロセスごとの CPU の数え方です。「1 コアで 100%」では、論理プロセッサ 1 個を使い切ると 100% になります(8 個なら最大 800%)。変わるのはプロセスごとの数字(表・詳細・使っているプロセスの上位・ミニ表示の一番のプロセス)で、PC 全体の CPU の使用率とグラフは、いつも 0〜100% です。絞り込みの cpu>50 も表の数字と同じ数え方で比べます";

pub const GRAPH_TEXT: &str = "グラフに持つ回数です。メモリの中だけに持ち、保存しません。長くすると、少しだけメモリを多く使います";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SPage {
    #[default]
    General,
    Look,
    Table,
    MiniTray,
    TouchFree,
    Diagnostics,
}

impl SPage {
    pub const ALL: [SPage; 6] = [SPage::General, SPage::Look, SPage::Table, SPage::MiniTray, SPage::TouchFree, SPage::Diagnostics];
    fn label(self) -> &'static str {
        match self {
            SPage::General => "全般",
            SPage::Look => "見た目",
            SPage::Table => "表とグラフ",
            SPage::MiniTray => "ミニ表示とトレイ",
            SPage::TouchFree => "触れないプロセス",
            SPage::Diagnostics => "診断",
        }
    }
}

/// enum の選択(segmented)。変わったら true
fn choose<T: PartialEq + Copy>(ui: &mut Ui, pal: &Palette, label: &str, value: &mut T, all: &[T], name: fn(T) -> &'static str) -> bool {
    let before = *value;
    ui.horizontal_wrapped(|ui| {
        if !label.is_empty() {
            ui.label(label);
        }
        let items: Vec<(T, String)> = all.iter().map(|&v| (v, name(v).to_string())).collect();
        segmented(ui, pal, value, &items);
    });
    *value != before
}

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    pub page: SPage,
    pub touch_input: String,
    pub touch_selected: Option<usize>,
}

/// 設定を開く。置き換えの値を読み直す(E-46)
pub fn open(st: &mut State) {
    st.settings.open = true;
    st.job(Job::ReplaceStatus);
}

/// 起動時と設定を開いたときの置き換えの状態(FR-56・E-44・E-46)
pub fn on_replace_info(st: &mut State, info: ReplaceInfo) {
    let status = taskmgr::status_of(info.debugger.as_deref(), &info.self_final_path);
    match &status {
        ReplaceStatus::OnSelf { same_path: false, .. } => {
            st.push_band(Band {
                kind: BandKind::Warn,
                key: BandKey::ReplaceMismatch,
                text: "Ctrl+Shift+Esc の登録先が、今の ProcSight と違います".into(),
                buttons: vec![("登録し直す", BandButton::ReplaceReRegister, info.block.is_none()), ("外す", BandButton::ReplaceRemove, st.env.elevated)],
                born: st.now,
                closable: false,
            });
        }
        ReplaceStatus::Off | ReplaceStatus::Other(_) if st.cfg.taskmgr_replace.enabled => {
            st.push_band(Band {
                kind: BandKind::Warn,
                key: BandKey::ReplaceGone,
                text: "Ctrl+Shift+Esc の置き換えが外れています。セキュリティ ソフトが消した可能性があります".into(),
                buttons: vec![("登録し直す", BandButton::ReplaceReRegister, info.block.is_none()), ("オフにする", BandButton::ReplaceOff, true)],
                born: st.now,
                closable: false,
            });
        }
        _ => {}
    }
    st.replace = Some(info);
}

pub fn on_replace_changed(st: &mut State, enabled: bool, previous: String, r: Result<(), OpError>) {
    match r {
        Ok(()) => {
            st.cfg.taskmgr_replace.enabled = enabled;
            // 登録し直し(E-44・E-46)では enable が前の値を返さない(今の値が ProcSight の物か空)。覚えた値を消さない(D-41)
            if !enabled {
                st.cfg.taskmgr_replace.previous_debugger.clear();
            } else if !previous.is_empty() {
                st.cfg.taskmgr_replace.previous_debugger = previous;
            }
            st.save_config();
            st.remove_band(BandKey::ReplaceGone);
            st.remove_band(BandKey::ReplaceMismatch);
            st.remove_band(BandKey::ReplaceRestore);
            st.ok(if enabled {
                "Ctrl+Shift+Esc で ProcSight が開くようにしました"
            } else {
                "Ctrl+Shift+Esc を Windows のタスク マネージャーに戻しました"
            });
            crate::log::info("replace", &format!("enabled={enabled} result=ok"));
        }
        // E-48: config の enabled は変えない
        Err(e) => {
            crate::log::warn("replace", &format!("enabled={enabled} result=failed"));
            st.fail(&e);
        }
    }
    st.job(Job::ReplaceStatus);
}

/// 左の並びの 1 項目。選んでいる物は薄い強調の地と、左の縦の棒
fn nav_item(ui: &mut Ui, pal: &Palette, selected: bool, text: &str) -> egui::Response {
    let h = (size::BODY + space::L * 1.5).round();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(4), widgets::tint(pal.accent, 30));
            let bar = Rect::from_min_size(rect.min + Vec2::new(0.0, space::S + 2.0), Vec2::new(3.0, h - (space::S + 2.0) * 2.0));
            p.rect_filled(bar, CornerRadius::same(2), pal.accent);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(4), ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let color = if selected { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
        p.text(egui::pos2(rect.left() + space::L, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(size::BODY), color);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, text));
    resp
}

/// 見出し(前に少し間を空ける)
fn head(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(space::M);
    widgets::section(ui, pal, title);
}

/// 弱い文字の説明(折り返す)
fn desc(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(pal.weak)).wrap());
}

pub fn window(st: &mut State, ui: &mut Ui) {
    if !st.settings.open {
        return;
    }
    let pal = theme::palette(ui);
    let mut open = true;
    let ctx = ui.ctx().clone();
    // 狭い窓・大きな文字でもはみ出さない大きさ(題の帯の分を引く)
    let screen = ctx.content_rect();
    let max = Vec2::new(screen.width() - 24.0, screen.height() - 64.0);
    egui::Window::new("設定")
        .open(&mut open)
        .default_size(Vec2::new(720.0, 520.0).min(max))
        .min_size(Vec2::new(680.0, 460.0).min(max))
        .max_size(max)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .collapsible(false)
        .show(&ctx, |ui| {
            let nav_w = if ui.available_width() < 560.0 { 124.0 } else { 160.0 };
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(nav_w);
                    ui.spacing_mut().item_spacing.y = space::XS;
                    for p in SPage::ALL {
                        if nav_item(ui, &pal, st.settings.page == p, p.label()).clicked() {
                            st.settings.page = p;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("settings_page").auto_shrink([false, false]).show(ui, |ui| {
                        ui.add_space(space::XS);
                        ui.label(RichText::new(st.settings.page.label()).size(size::TITLE).strong());
                        match st.settings.page {
                            SPage::General => general(st, ui),
                            SPage::Look => look(st, ui),
                            SPage::Table => table(st, ui),
                            SPage::MiniTray => mini_tray(st, ui),
                            SPage::TouchFree => touch_free(st, ui),
                            SPage::Diagnostics => diagnostics(st, ui),
                        }
                        ui.add_space(space::L);
                    });
                });
            });
        });
    if !open {
        st.settings.open = false;
    }
}

fn general(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    head(ui, &pal, "更新間隔");
    let mut ms = st.cfg.interval_ms;
    ui.horizontal_wrapped(|ui| {
        for v in INTERVALS_MS {
            ui.radio_value(&mut ms, v, crate::app::interval_label(v));
        }
    });
    if ms != st.cfg.interval_ms {
        st.set_interval(ms);
    }
    slow_section(st, ui, &pal);
    head(ui, &pal, "表示");
    let mut top = st.cfg.always_on_top;
    if ui.checkbox(&mut top, "常に手前に表示").changed() {
        st.set_always_on_top(top);
    }
    desc(ui, &pal, ALWAYS_ON_TOP_TEXT);
    head(ui, &pal, "ウィンドウを閉じたとき");
    let mut ca = st.cfg.effective_close_action();
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut ca, CloseAction::Exit, "終了する");
        let r = ui.add_enabled(st.cfg.tray_enabled, egui::RadioButton::new(ca == CloseAction::Tray, "トレイに残す"));
        if r.clicked() {
            ca = CloseAction::Tray;
        }
        r.on_disabled_hover_text("トレイにアイコンを出すときだけ選べます");
    });
    if ca != st.cfg.effective_close_action() {
        st.cfg.close_action = ca;
        st.save_config();
    }
    if ui.checkbox(&mut st.cfg.tray_enabled, "トレイにアイコンを出す").changed() {
        st.save_config();
        st.actions.push(Action::TrayChanged);
    }
    head(ui, &pal, "起動");
    let mut changed = choose(ui, &pal, "開くタブ", &mut st.cfg.start_tab, StartTab::ALL, StartTab::label);
    let shown = st.cfg.effective_start_mode();
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("起動したときの窓");
        for &m in StartMode::ALL {
            let enabled = m != StartMode::Tray || st.cfg.tray_enabled;
            let r = ui.add_enabled(enabled, egui::RadioButton::new(shown == m, m.label()));
            if r.clicked() {
                clicked = Some(m);
            }
            if !enabled {
                r.on_disabled_hover_text("トレイにアイコンを出すときだけ選べます");
            }
        }
    });
    // 押した物をそのまま入れる(トレイを切っている間に残っていた Tray も、「普通の表示」を押せば消える)
    if let Some(m) = clicked.filter(|m| *m != st.cfg.start_mode) {
        st.cfg.start_mode = m;
        changed = true;
    }
    desc(ui, &pal, "Ctrl+Shift+Esc で開いたときと、管理者で再起動したときは、トレイにしまわずに窓を出します");
    if changed {
        st.save_config();
    }
    if ui.checkbox(&mut st.cfg.elevate_on_start, "起動時に管理者へ切り替える").changed() {
        st.save_config();
    }
    desc(ui, &pal, "オンにすると、起動のたびに UAC の確認が出ます");
    replace_section(st, ui);
}

/// 触れないプロセスが動いている間の遅い更新(v0.3.0。既定オフ)。変えたらすぐ判定し直す
fn slow_section(st: &mut State, ui: &mut Ui, pal: &Palette) {
    ui.add_space(space::S);
    let mut changed = ui.checkbox(&mut st.cfg.slow_when_touch_free, "触れないプロセスが動いている間は更新を遅くする").changed();
    desc(ui, pal, SLOW_TEXT);
    let on = st.cfg.slow_when_touch_free;
    let mut slow = st.cfg.slow_interval_ms;
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(on, |ui| {
            ui.label("遅くするときの間隔:");
            for v in INTERVALS_MS.into_iter().filter(|v| *v >= 1000) {
                ui.radio_value(&mut slow, v, crate::app::interval_label(v));
            }
        });
    });
    if slow != st.cfg.slow_interval_ms {
        st.cfg.slow_interval_ms = slow;
        changed = true;
    }
    if on && st.cfg.slow_interval_ms <= st.cfg.interval_ms {
        desc(ui, pal, "今の更新間隔のほうが長いか同じため、遅くはなりません");
    }
    if changed {
        st.save_config();
        st.update_slow_down();
    }
}

fn replace_section(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    head(ui, &pal, "Ctrl+Shift+Esc の置き換え");
    let info = st.replace.clone();
    let on = st.cfg.taskmgr_replace.enabled;
    // 灰色の理由(D-39・FR-53)
    let block = if !st.env.elevated {
        Some(taskmgr::ReplaceBlock::NotElevated)
    } else {
        info.as_ref().and_then(|i| i.block).or_else(|| taskmgr::check_location(&st.env.self_final_path, &st.env.program_files).err())
    };
    let mut want = on;
    let r = ui.add_enabled(block.is_none() && info.is_some(), egui::Checkbox::new(&mut want, "Ctrl+Shift+Esc で ProcSight を開く"));
    if let Some(b) = block {
        r.on_disabled_hover_text(b.message());
        ui.add(egui::Label::new(RichText::new(b.message()).color(pal.warn)).wrap());
    }
    if want != on {
        if want {
            let current = info.as_ref().and_then(|i| match taskmgr::status_of(i.debugger.as_deref(), &i.self_final_path) {
                ReplaceStatus::Other(v) => Some(v),
                _ => None,
            });
            st.dialog = Some(Dialog::ReplaceOn { current, opened: false });
        } else {
            st.job(Job::ReplaceDisable { previous: st.cfg.taskmgr_replace.previous_debugger.clone() });
        }
    }
    let (status, color) = match &info {
        None => ("読み込んでいます".to_string(), pal.weak),
        Some(i) => match taskmgr::status_of(i.debugger.as_deref(), &i.self_final_path) {
            ReplaceStatus::OnSelf { target, .. } => (format!("オン(登録先: {target})"), pal.ok),
            _ if on => ("外れています".into(), pal.warn),
            ReplaceStatus::Off => ("オフ".into(), pal.weak),
            ReplaceStatus::Other(v) => (format!("オフ(別の道具の登録: {v})"), pal.weak),
        },
    };
    ui.horizontal_wrapped(|ui| {
        ui.label("状態:");
        ui.label(RichText::new(status).color(color));
    });
    if let Some(v) = info.as_ref().and_then(|i| i.debugger_wow64.clone()).filter(|v| !v.trim().is_empty()) {
        desc(ui, &pal, &format!("32 ビット側にも登録があります: {v}(ProcSight は変えません)"));
    }
    ui.add_space(space::S);
    desc(ui, &pal, REPLACE_TEXT);
}

fn look(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    head(ui, &pal, "テーマ");
    let mut t = st.cfg.theme;
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut t, Theme::Dark, "ダーク");
        ui.radio_value(&mut t, Theme::Light, "ライト");
        ui.radio_value(&mut t, Theme::System, "Windows に合わせる");
    });
    if t != st.cfg.theme {
        st.cfg.theme = t;
        st.save_config();
    }
    head(ui, &pal, "強調の色");
    if accent_picker(ui, &mut st.cfg.accent) {
        st.save_config();
    }
    head(ui, &pal, "文字");
    let mut pct = (st.cfg.font_scale * 100.0).round() as i32;
    ui.add(egui::Slider::new(&mut pct, 80..=160).step_by(10.0).suffix("%").text("文字の大きさ"));
    let scale = pct as f32 / 100.0;
    if (scale - st.cfg.font_scale).abs() > 0.001 {
        st.cfg.font_scale = scale;
        st.save_config();
    }
    head(ui, &pal, "一覧の表示");
    let mut v = st.cfg.view;
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut v, View::Tree, "ツリー");
        ui.radio_value(&mut v, View::Flat, "一覧");
    });
    if v != st.cfg.view {
        st.cfg.view = v;
        st.save_config();
    }
    head(ui, &pal, "表");
    let mut d = st.cfg.density;
    ui.horizontal(|ui| {
        ui.label("行の高さ");
        let items: Vec<(Density, String)> = Density::ALL.iter().map(|&x| (x, x.label().to_string())).collect();
        segmented(ui, &pal, &mut d, &items);
    });
    if d != st.cfg.density {
        st.cfg.density = d;
        st.save_config();
    }
    ui.add_space(space::S);
    if ui.checkbox(&mut st.cfg.heat_map, "使用量で色を塗る").changed() {
        st.save_config();
    }
    desc(ui, &pal, HEAT_MAP_TEXT);
    ui.add_enabled_ui(st.cfg.heat_map, |ui| {
        let mut changed = choose(ui, &pal, "塗り始める所", &mut st.cfg.heat_start, HeatStart::ALL, HeatStart::label);
        changed |= choose(ui, &pal, "一番濃くする所", &mut st.cfg.heat_full, HeatFull::ALL, HeatFull::label);
        if changed {
            st.save_config();
        }
    });
}

/// 強調の色の丸(選んでいる物に輪)。変わったら true
fn accent_picker(ui: &mut Ui, value: &mut Accent) -> bool {
    let dark = ui.visuals().dark_mode;
    let before = *value;
    ui.horizontal_wrapped(|ui| {
        for &a in Accent::ALL {
            let color = theme::palette_for(dark, a).accent;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                p.circle_filled(rect.center(), 9.0, color);
                if *value == a {
                    p.circle_stroke(rect.center(), 12.0, egui::Stroke::new(2.0, ui.visuals().strong_text_color()));
                } else if resp.hovered() {
                    p.circle_stroke(rect.center(), 12.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()));
                }
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, *value == a, a.label()));
            if resp.on_hover_text(a.label()).clicked() {
                *value = a;
            }
        }
    });
    *value != before
}

/// 表とグラフ(v0.4.0): CPU の数え方・速さの単位・グラフの長さ・ダブルクリック・ツリーの最初の開き方
fn table(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    let mut changed = false;
    head(ui, &pal, "CPU の数え方");
    changed |= choose(ui, &pal, "", &mut st.cfg.cpu_scale, CpuScale::ALL, CpuScale::label);
    desc(ui, &pal, CPU_SCALE_TEXT);
    head(ui, &pal, "速さの単位");
    changed |= choose(ui, &pal, "", &mut st.cfg.speed_unit, SpeedUnit::ALL, SpeedUnit::label);
    desc(ui, &pal, "ディスクと通信の速さに使います。ビットは 1000 進です(1 Mbps = 1,000,000 ビット毎秒)");
    head(ui, &pal, "グラフの長さ");
    let mut n = st.cfg.graph_points;
    ui.horizontal_wrapped(|ui| {
        for v in GRAPH_POINTS_CHOICES {
            let span = crate::perf::span_text(st.cfg.interval_ms * v as u64);
            ui.radio_value(&mut n, v, format!("{v} 回(今の間隔で {span})"));
        }
    });
    if n != st.cfg.graph_points {
        st.cfg.graph_points = n;
        changed = true;
    }
    desc(ui, &pal, GRAPH_TEXT);
    head(ui, &pal, "プロセスの表");
    changed |= choose(ui, &pal, "行のダブルクリック", &mut st.cfg.double_click, DoubleClick::ALL, DoubleClick::label);
    changed |= choose(ui, &pal, "ツリーで新しく見えたプロセス", &mut st.cfg.tree_start, TreeStart::ALL, TreeStart::label);
    desc(ui, &pal, "「全部畳む」では、新しく見えたプロセスを畳んだ状態で出します。今出ているプロセスは変わりません(次の起動から全部に効きます)。絞り込んでいる間は、一致した行が見えるように開いて出します。開いた・畳んだ状態は保存しません");
    if changed {
        st.save_config();
    }
}

/// ミニ表示とトレイ(v0.4.0): ミニ表示に出す物、トレイのアイコンの値
fn mini_tray(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    head(ui, &pal, "ミニ表示に出す物");
    let m = &mut st.cfg.mini_items;
    let before = m.clone();
    ui.horizontal_wrapped(|ui| {
        for (v, label) in [(&mut m.cpu, "CPU"), (&mut m.mem, "メモリ"), (&mut m.gpu, "GPU"), (&mut m.disk, "ディスク"), (&mut m.net_recv, "受信"), (&mut m.net_send, "送信")] {
            ui.checkbox(v, label);
        }
    });
    ui.checkbox(&mut m.top_process, "CPU を一番使っているプロセス");
    // 全部は消させない(最後の 1 つは戻す)
    if !m.any_graph() && !m.top_process {
        *m = before.clone();
    }
    if *m != before {
        st.save_config();
    }
    desc(ui, &pal, "全部は外せません(最後の 1 つは残ります)。ミニ表示は Ctrl+M か、メニューの「表示 > ミニ表示」で切り替えます");
    head(ui, &pal, "トレイのアイコン");
    let mut v = st.cfg.tray_value;
    ui.add_enabled_ui(st.cfg.tray_enabled, |ui| {
        if choose(ui, &pal, "メーターに出す値", &mut v, TrayValue::ALL, TrayValue::label) {
            st.cfg.tray_value = v;
            st.save_config();
        }
    });
    if !st.cfg.tray_enabled {
        desc(ui, &pal, "トレイにアイコンを出すとき(「全般」)だけ選べます");
    }
}

fn touch_free(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    ui.add_space(space::S);
    ui.add(egui::Label::new(TOUCH_FREE_TEXT).wrap());
    head(ui, &pal, "登録した exe");
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut st.settings.touch_input).hint_text("game.exe").desired_width(220.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let full = st.cfg.touch_free.names.len() >= MAX_TOUCH_FREE;
        let add = ui.add_enabled(!full, egui::Button::new("追加")).on_disabled_hover_text("100 個までです");
        if add.clicked() || (enter && !full) {
            // 1 行に 1 つ。貼り付けた複数の行も受ける(FR-21・E-39)
            let lines: Vec<String> = st.settings.touch_input.lines().map(str::to_string).collect();
            let mut names = st.cfg.touch_free.names.clone();
            names.extend(lines);
            let names = rules::normalize_touch_free_list(names.iter().map(|s| s.as_str()));
            if names != st.cfg.touch_free.names {
                st.cfg.touch_free.names = names;
                changed = true;
            }
            st.settings.touch_input.clear();
        }
        let del = ui.add_enabled(st.settings.touch_selected.is_some(), egui::Button::new("削除"));
        if del.clicked()
            && let Some(i) = st.settings.touch_selected.take()
            && i < st.cfg.touch_free.names.len()
        {
            st.cfg.touch_free.names.remove(i);
            changed = true;
        }
        ui.label(RichText::new(format!("{} / {MAX_TOUCH_FREE}", st.cfg.touch_free.names.len())).color(pal.weak).size(size::SMALL));
    });
    ui.add_space(space::S);
    widgets::card(ui, &pal, |ui| {
        ui.set_min_height(120.0);
        ui.set_width(ui.available_width());
        if st.cfg.touch_free.names.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(space::XL);
                ui.label(RichText::new("登録はありません").color(pal.weak));
            });
        }
        egui::ScrollArea::vertical().id_salt("touch_free_list").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
            for (i, n) in st.cfg.touch_free.names.iter().enumerate() {
                if ui.selectable_label(st.settings.touch_selected == Some(i), n).clicked() {
                    st.settings.touch_selected = Some(i);
                }
            }
        });
    });
    ui.add_space(space::S);
    if ui.checkbox(&mut st.cfg.touch_free.include_descendants, "子孫も含める").changed() {
        changed = true;
    }
    let running = st.snap.touch_free.len();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, if running > 0 { pal.accent } else { pal.weak });
        ui.label(format!("今動いている触れないプロセス: {running} 個"));
    });
    if changed {
        st.save_config();
        st.actions.push(Action::SetTouchFree(st.cfg.touch_free_set()));
        crate::log::info("touch_free", &format!("count={} descendants={}", st.cfg.touch_free.names.len(), st.cfg.touch_free.include_descendants));
    }
}

/// 中央値・95 パーセンタイル・最大
pub fn stats(v: &[f32]) -> Option<(f32, f32, f32)> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f32| s[((s.len() - 1) as f32 * q).round() as usize];
    Some((at(0.5), at(0.95), s[s.len() - 1]))
}

fn stats_text(v: &[f32]) -> String {
    stats(v).map_or_else(|| format::DASH.into(), |(m, p, x)| format!("中央値 {m:.1} ms ・ 95% {p:.1} ms ・ 最大 {x:.1} ms"))
}

fn diagnostics(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    let d = st.diag.clone();
    head(ui, &pal, "ProcessGate の記録");
    // 触れないプロセスの行を先頭に
    let mut rows: Vec<(u32, String, u32, bool)> = d
        .open_counts
        .iter()
        .map(|(pid, n)| {
            let r = st.snap.find_pid(*pid);
            (*pid, r.map_or_else(|| format::DASH.to_string(), |r| r.name.clone()), *n, r.is_some_and(|r| r.touch_free))
        })
        .collect();
    for id in &st.snap.touch_free {
        if !rows.iter().any(|r| r.0 == id.pid) {
            rows.push((id.pid, st.snap.find(*id).map_or_else(String::new, |r| r.name.clone()), 0, true));
        }
    }
    rows.sort_by_key(|r| (!r.3, std::cmp::Reverse(r.2), r.0));
    let touch_opens: u32 = rows.iter().filter(|r| r.3).map(|r| r.2).sum();
    let c = if touch_opens > 0 { pal.danger } else { pal.ok };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, c);
        ui.label(RichText::new(format!("触れないプロセスへの開いた回数: {touch_opens}")).color(if touch_opens > 0 { pal.danger } else { ui.visuals().text_color() }));
    });
    ui.add_space(space::S);
    widgets::card(ui, &pal, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("gate").max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("gate_grid").num_columns(4).striped(true).spacing([space::XL, space::S]).show(ui, |ui| {
                for h in ["PID", "名前", "開いた回数", "触れない"] {
                    ui.label(RichText::new(h).color(pal.weak).size(size::SMALL));
                }
                ui.end_row();
                for (pid, name, n, tf) in rows.iter().take(500) {
                    let color = if *tf && *n > 0 { pal.danger } else { ui.visuals().text_color() };
                    ui.label(RichText::new(pid.to_string()).color(color));
                    ui.label(RichText::new(name).color(color));
                    ui.label(RichText::new(n.to_string()).color(color));
                    ui.label(RichText::new(if *tf { "はい" } else { "" }).color(color));
                    ui.end_row();
                }
            });
        });
    });
    head(ui, &pal, "時間の記録");
    let mut timing = d.timing;
    if ui.checkbox(&mut timing, "時間を記録する").changed() {
        st.actions.push(Action::SetTiming(timing));
        st.diag.timing = timing;
    }
    if timing {
        ui.label(format!("取得の 1 周: {}", stats_text(&d.collect_ms)));
        ui.label(format!("1 フレーム: {}", stats_text(&st.frame_ms)));
    }
    head(ui, &pal, "公開されていない API");
    let a = &d.api;
    egui::Grid::new("api").num_columns(2).spacing([space::XL, space::S]).show(ui, |ui| {
        for (k, v) in [
            ("CPU 時間・開始時刻(事実19)", a.spi_times),
            ("プライベート作業セット(事実19)", a.spi_private_ws),
            ("I/O の量(事実19)", a.spi_io),
            ("一時停止と再開(事実21)", a.suspend),
            ("ハンドルの一覧(事実22)", a.handles),
            ("コマンドライン(事実23)", a.cmdline),
            ("PDH の CPU", a.pdh_cpu),
            ("PDH のディスク", a.pdh_disk),
            ("PDH の GPU", a.pdh_gpu),
        ] {
            ui.label(k);
            widgets::dot_label(ui, if v { "使える" } else { "使えない" }, if v { pal.ok } else { pal.warn });
            ui.end_row();
        }
    });
    head(ui, &pal, "フォント");
    match &st.font {
        Some(f) => ui.add(egui::Label::new(format!("{}(書体の番号 {}: {})", f.path, f.index, f.face)).wrap()),
        None => ui.label(RichText::new("読めませんでした(Windows の補いだけで表示しています)").color(pal.warn)),
    };
    head(ui, &pal, "置き換え");
    let (dbg, dbg32) = st.replace.as_ref().map_or((None, None), |i| (i.debugger.clone(), i.debugger_wow64.clone()));
    egui::Grid::new("replace_diag").num_columns(2).spacing([space::XL, space::S]).show(ui, |ui| {
        let k = |ui: &mut Ui, s: &str| ui.label(RichText::new(s).color(pal.weak));
        k(ui, "Debugger(普段のビュー)");
        ui.label(dbg.unwrap_or_else(|| "(なし)".into()));
        ui.end_row();
        k(ui, "Debugger(WOW6432Node)");
        ui.label(dbg32.unwrap_or_else(|| "(なし)".into()));
        ui.end_row();
        k(ui, "config.toml の enabled");
        ui.label(st.cfg.taskmgr_replace.enabled.to_string());
        ui.end_row();
        k(ui, "config.toml の previous_debugger");
        ui.label(if st.cfg.taskmgr_replace.previous_debugger.is_empty() { "(空)".to_string() } else { st.cfg.taskmgr_replace.previous_debugger.clone() });
        ui.end_row();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let v: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        let (m, p, x) = stats(&v).unwrap();
        assert!((50.0..=51.0).contains(&m) && (95.0..=96.0).contains(&p) && x == 100.0);
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn replace_gone_band_when_enabled_but_value_missing() {
        // E-46
        let mut st = crate::demo::state();
        st.cfg.taskmgr_replace.enabled = true;
        on_replace_info(&mut st, ReplaceInfo { debugger: None, debugger_wow64: None, self_final_path: "C:\\Program Files\\ProcSight\\ProcSight.exe".into(), block: None });
        assert!(st.bands.iter().any(|b| b.key == BandKey::ReplaceGone));
        // E-44: 別の場所の ProcSight を指している
        let mut st = crate::demo::state();
        on_replace_info(&mut st, ReplaceInfo {
            debugger: Some("\"C:\\Program Files\\Old\\ProcSight.exe\"".into()),
            debugger_wow64: None,
            self_final_path: "C:\\Program Files\\ProcSight\\ProcSight.exe".into(),
            block: None,
        });
        assert!(st.bands.iter().any(|b| b.key == BandKey::ReplaceMismatch));
    }

    #[test]
    fn replace_failure_keeps_enabled_flag() {
        // E-48
        let mut st = crate::demo::state();
        on_replace_changed(&mut st, true, String::new(), Err(OpError::Win32(5)));
        assert!(!st.cfg.taskmgr_replace.enabled);
        on_replace_changed(&mut st, true, "\"C:\\x\\procexp.exe\"".into(), Ok(()));
        assert!(st.cfg.taskmgr_replace.enabled && st.config_dirty);
        assert_eq!(st.cfg.taskmgr_replace.previous_debugger, "\"C:\\x\\procexp.exe\"");
    }
}
