//! S1 プロセス(画面設計書 §4): 絞り込み・範囲・ツリーと一覧・列・並べ替え・色・アイコン・右クリックのメニュー・キーボード。
//! 表は egui_extras で見える行だけを描く(D-1・NFR-4)。並びは Snapshot・絞り込み・並べ替え・畳みが変わったときだけ作り直す。
//! 終了・一時停止は確認の窓(D1・D2)を開くだけで、Job は積まない(INV-5)。

use eframe::egui;
use crate::app::{IconSlot, State};
use crate::dialogs::Dialog;
use crate::theme::{self, Palette};
use crate::{details, log};
use egui::{Align, Color32, Key, Layout, Modifiers, RichText, Sense, Ui, Vec2};
use egui_extras::TableBuilder;
use procsight_core::config::{Config, View};
use procsight_core::filter::{self, Scope, SortValue};
use procsight_core::format;
use procsight_core::rules;
use procsight_core::tree::{self, Forest, VisibleRow};
use procsight_core::{Column, Priority, ProcId, ProcState, ProcessRow, Snapshot};
use procsight_win::jobs::{Job, Target};
use std::collections::HashSet;

pub const ROW_H: f32 = 20.0;

#[derive(Clone, Debug, PartialEq)]
struct ListKey {
    seq: u64,
    filter: String,
    scope: Scope,
    col: Column,
    desc: bool,
    tree: bool,
    generation: u64,
}

pub struct ProcsView {
    pub filter: String,
    pub scope: Scope,
    pub focus_filter: bool,
    pub selected: Vec<ProcId>,
    anchor: Option<ProcId>,
    pub collapsed: HashSet<ProcId>,
    key: Option<ListKey>,
    generation: u64,
    pub visible: Vec<VisibleRow>,
    forest: Forest,
    /// FR-44・S8 から: この行を選んで見える位置まで動かす
    pub scroll_to: Option<ProcId>,
    pub detail: details::DetailState,
}

impl ProcsView {
    pub fn new(_cfg: &Config) -> ProcsView {
        ProcsView {
            filter: String::new(),
            scope: Scope::All,
            focus_filter: false,
            selected: Vec::new(),
            anchor: None,
            collapsed: HashSet::new(),
            key: None,
            generation: 0,
            visible: Vec::new(),
            forest: Forest::default(),
            scroll_to: None,
            detail: details::DetailState::default(),
        }
    }

    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    pub fn on_snapshot(&mut self, snap: &Snapshot) {
        self.detail.push_history(snap);
    }

    /// 1 つだけ選んでいるときの行
    pub fn single(&self) -> Option<ProcId> {
        (self.selected.len() == 1).then(|| self.selected[0])
    }

    pub fn select_only(&mut self, id: ProcId) {
        if self.selected != [id] {
            self.selected = vec![id];
            self.detail.reset(id);
        }
        self.anchor = Some(id);
    }
}

/// 名前の列に出す文字(PID 0 は diff が名付け済み。自分には「(ProcSight)」。E-40)
pub fn display_name(st: &State, r: &ProcessRow) -> String {
    if r.id.pid == st.self_pid() && r.id.pid != 0 { format!("{}(ProcSight)", r.name) } else { r.name.clone() }
}

fn sig_text(st: &State, r: &ProcessRow) -> String {
    match r.path().and_then(|p| st.sigs.get(p)) {
        None => format::DASH.into(),
        Some(None) => "確かめています".into(),
        Some(Some(s)) => s.status.label(),
    }
}

pub fn column_title(st: &State, c: Column) -> &'static str {
    // D-10: プライベート作業セットが取れないときは「メモリ」は「コミット」と同じ意味
    if c == Column::Mem && st.snap.seq > 0 && !st.diag.api.spi_private_ws { "コミット" } else { c.title() }
}

/// 表のセルとコピーに使う文字
pub fn cell_text(st: &State, r: &ProcessRow, c: Column) -> String {
    match c {
        Column::Name => display_name(st, r),
        Column::Pid => r.id.pid.to_string(),
        Column::State => r.state.label().into(),
        Column::Cpu => format::pct(r.cpu_pct),
        Column::Mem => format::opt_bytes(r.mem),
        Column::WorkingSet => format::bytes(r.working_set),
        Column::Commit => format::bytes(r.commit),
        Column::IoRead => format::speed(r.io_read_bps),
        Column::IoWrite => format::speed(r.io_write_bps),
        Column::Gpu => format::pct(r.gpu_pct),
        Column::Threads => format::thousands(r.threads as u64),
        Column::Handles => format::thousands(r.handles as u64),
        Column::User => r.user().unwrap_or(format::DASH).into(),
        Column::Priority => Priority::base_label(r.base_priority),
        Column::Session => r.session_id.to_string(),
        Column::StartTime => format::filetime(r.id.create_time, st.env.utc_offset_minutes),
        Column::Path => r.path().unwrap_or(format::DASH).into(),
        Column::Signature => sig_text(st, r),
        // E-6: 親がいない・使い回しなら「(終了)」を添える
        Column::ParentPid if r.parent.is_none() && r.parent_pid != 0 && r.parent_pid != r.id.pid => {
            format!("{}(終了)", r.parent_pid)
        }
        Column::ParentPid => r.parent_pid.to_string(),
    }
}

fn sort_value(st: &State, r: &ProcessRow, c: Column) -> SortValue {
    if c == Column::Signature {
        return match r.path().and_then(|p| st.sigs.get(p)) {
            Some(Some(s)) => SortValue::Text(s.status.label()),
            _ => SortValue::None,
        };
    }
    filter::column_value(r, c)
}

/// 並びを必要なときだけ作り直す
fn refresh(st: &mut State) {
    let key = ListKey {
        seq: st.snap.seq,
        filter: st.procs.filter.clone(),
        scope: st.procs.scope,
        col: st.cfg.columns.sort_column(),
        desc: st.cfg.columns.sort.descending,
        tree: st.cfg.view == View::Tree,
        generation: st.procs.generation,
    };
    if st.procs.key.as_ref() == Some(&key) {
        return;
    }
    let snap = st.snap.clone();
    let rows = &snap.rows;
    let me = st.env.user.clone();
    let (q, scope, col, desc) = (key.filter.clone(), key.scope, key.col, key.desc);
    let st_ro: &State = st;
    let m = |r: &ProcessRow| filter::matches(r, &q, scope, me.as_deref());
    let cmp = |a: &ProcessRow, b: &ProcessRow| {
        filter::compare_values(&sort_value(st_ro, a, col), &sort_value(st_ro, b, col), desc, a.id.pid, b.id.pid)
    };
    let (forest, visible) = if key.tree {
        let f = tree::build(rows);
        let v = tree::flatten_tree(rows, &f, &st_ro.procs.collapsed, &m, &cmp);
        (f, v)
    } else {
        (Forest::default(), tree::flatten_list(rows, &m, &cmp))
    };
    st.procs.forest = forest;
    st.procs.visible = visible;
    st.procs.key = Some(key);
}

/// 右クリックのメニューと Delete・Ctrl+C から出す頼み事
#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Terminate(Vec<ProcId>),
    TerminateTree(ProcId),
    Suspend(ProcId),
    Resume(ProcId),
    Priority(ProcId, Priority),
    Efficiency(ProcId, bool),
    Affinity(ProcId),
    Front(ProcId),
    OpenLocation(String),
    Properties(String),
    Copy(Vec<ProcId>),
    OpenTaskmgr,
}

#[derive(Default)]
struct Events {
    click: Option<(ProcId, Modifiers)>,
    secondary: Option<ProcId>,
    double: Option<ProcId>,
    toggle: Option<ProcId>,
    sort: Option<Column>,
    cmds: Vec<Cmd>,
    icons: Vec<String>,
    sigs: Vec<String>,
    widths: Option<Vec<f32>>,
}

/// 終了の灰色の理由(INV-4・触れない・終了処理中)
pub fn terminate_refusal(st: &State, r: &ProcessRow) -> Option<String> {
    if r.state == ProcState::Exiting && !r.touch_free {
        return Some("終了処理中のプロセスです".into());
    }
    rules::check_terminate(r, st.self_pid(), &st.env.system_root).err().map(|x| x.message().to_string())
}

pub fn suspend_refusal(st: &State, r: &ProcessRow) -> Option<String> {
    if r.state == ProcState::Exiting && !r.touch_free {
        return Some("終了処理中のプロセスです".into());
    }
    rules::check_suspend(r, st.self_pid(), &st.env.system_root).err().map(|x| x.message().to_string())
}

/// 操作の全般の灰色の理由(触れない・終了処理中・終わった)
fn touch_refusal(r: &ProcessRow) -> Option<&'static str> {
    if r.touch_free {
        Some("触れないプロセスに登録されています")
    } else if r.is_gone > 0 {
        Some("すでに終了しています")
    } else if r.state == ProcState::Exiting {
        Some("終了処理中のプロセスです")
    } else if r.is_idle_or_system() {
        // E-8: PID 0 と 4 はメニューを全部灰色に
        Some("Windows の中核のプロセスのため、操作できません")
    } else {
        None
    }
}

fn item(ui: &mut Ui, label: &str, refusal: Option<&str>) -> bool {
    item_key(ui, label, "", refusal)
}

/// キーの表示つき(画面設計書 §4.3 の「終了(Delete)」「コピー(Ctrl+C)」)
fn item_key(ui: &mut Ui, label: &str, key: &str, refusal: Option<&str>) -> bool {
    let r = ui.add_enabled(refusal.is_none(), egui::Button::new(label).shortcut_text(key));
    let r = match refusal {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    r.clicked()
}

fn context_menu(st: &State, ui: &mut Ui, r: &ProcessRow, targets: &[ProcId], out: &mut Vec<Cmd>) {
    let pal = theme::palette(ui);
    if r.touch_free && targets.len() == 1 {
        ui.label(RichText::new("触れないプロセスに登録されているため、ProcSight からは操作しません").color(pal.weak));
        ui.separator();
        let refusal = (st.cfg.taskmgr_replace.enabled && !st.env.elevated).then_some("管理者で再起動すると開けます");
        if item(ui, "Windows のタスク マネージャーを開く", refusal) {
            out.push(Cmd::OpenTaskmgr);
        }
        if item_key(ui, "コピー", "Ctrl+C", None) {
            out.push(Cmd::Copy(targets.to_vec()));
        }
        return;
    }
    if targets.len() > 1 {
        // FR-18: 複数のときは「終了」と「コピー」だけ
        if item_key(ui, &format!("終了({} 個)", targets.len()), "Delete", None) {
            out.push(Cmd::Terminate(targets.to_vec()));
        }
        if item_key(ui, "コピー", "Ctrl+C", None) {
            out.push(Cmd::Copy(targets.to_vec()));
        }
        return;
    }
    let id = r.id;
    let term = terminate_refusal(st, r);
    if item_key(ui, "終了", "Delete", term.as_deref()) {
        out.push(Cmd::Terminate(vec![id]));
    }
    if item(ui, "ツリーを終了", term.as_deref()) {
        out.push(Cmd::TerminateTree(id));
    }
    // R-2: 一時停止が使えないなら項目ごと出さない
    if st.diag.api.suspend {
        if r.state == ProcState::Suspended {
            if item(ui, "再開", touch_refusal(r)) {
                out.push(Cmd::Resume(id));
            }
        } else if item(ui, "一時停止", suspend_refusal(st, r).as_deref()) {
            out.push(Cmd::Suspend(id));
        }
    }
    let touch = touch_refusal(r);
    ui.add_enabled_ui(touch.is_none(), |ui| {
        ui.menu_button("優先度", |ui| {
            let now = Priority::from_base(r.base_priority);
            for p in Priority::ALL {
                if ui.radio(now == Some(p), p.label()).clicked() {
                    out.push(Cmd::Priority(id, p));
                    ui.close();
                }
            }
        })
        .response
        .on_disabled_hover_text(touch.unwrap_or_default());
    });
    // FR-14: 状態が読めたら片方、読めなければ両方
    match details::efficiency_of(st, id) {
        Some(true) => {
            if item(ui, "効率モードを切る", touch) {
                out.push(Cmd::Efficiency(id, false));
            }
        }
        Some(false) => {
            if item(ui, "効率モードを入れる", touch) {
                out.push(Cmd::Efficiency(id, true));
            }
        }
        None => {
            if item(ui, "効率モードを入れる", touch) {
                out.push(Cmd::Efficiency(id, true));
            }
            if item(ui, "効率モードを切る", touch) {
                out.push(Cmd::Efficiency(id, false));
            }
        }
    }
    let aff = touch.or((st.env.processor_groups > 1).then_some("この PC では使えません(プロセッサのグループが複数)"));
    if item(ui, "CPU の割り当て…", aff) {
        out.push(Cmd::Affinity(id));
    }
    let front = touch.or((!r.has_window).then_some("見えている窓がありません"));
    if item(ui, "前面に出す", front) {
        out.push(Cmd::Front(id));
    }
    let path = r.path().map(str::to_string);
    let no_path = touch.or(path.is_none().then_some("パスを取得できていません"));
    if item(ui, "ファイルの場所を開く", no_path) {
        out.push(Cmd::OpenLocation(path.clone().unwrap_or_default()));
    }
    if item(ui, "プロパティ", no_path) {
        out.push(Cmd::Properties(path.unwrap_or_default()));
    }
    ui.separator();
    if item_key(ui, "コピー", "Ctrl+C", None) {
        out.push(Cmd::Copy(targets.to_vec()));
    }
}

/// 見えている列の見出しの行と値の行(タブ区切り。FR-36)
pub fn copy_text(st: &State, ids: &[ProcId]) -> String {
    let cols = st.cfg.columns.visible_columns();
    let mut out: Vec<String> = vec![cols.iter().map(|c| column_title(st, *c)).collect::<Vec<_>>().join("\t")];
    for id in ids {
        if let Some(r) = st.snap.find(*id) {
            out.push(cols.iter().map(|c| cell_text(st, r, *c)).collect::<Vec<_>>().join("\t"));
        }
    }
    out.join("\r\n")
}

fn targets(st: &State, ids: &[ProcId]) -> Vec<Target> {
    ids.iter().filter_map(|id| st.target_of(*id)).collect()
}

/// 終了の確認を開く。灰色の条件の物があれば帯に理由を出すだけ(画面設計書 §8・E-5)
fn ask_terminate(st: &mut State, ids: &[ProcId]) {
    let mut ok = Vec::new();
    for id in ids {
        let Some(r) = st.snap.find(*id) else { continue };
        if let Some(why) = terminate_refusal(st, r) {
            let msg = if ids.len() > 1 { format!("{}(PID {}): {why}", r.name, r.id.pid) } else { why };
            st.band(crate::app::BandKind::Danger, msg);
            return;
        }
        ok.push(*id);
    }
    if !ok.is_empty() {
        st.dialog = Some(Dialog::Confirm { suspend: false, targets: targets(st, &ok), opened: false });
    }
}

fn run_cmd(st: &mut State, ui: &Ui, cmd: Cmd) {
    match cmd {
        Cmd::Terminate(ids) => ask_terminate(st, &ids),
        Cmd::TerminateTree(id) => {
            let (kill, excluded) = rules::collect_tree_for_terminate(&st.snap, id, st.self_pid(), &st.env.system_root);
            let kill = targets(st, &kill);
            let excluded = excluded
                .into_iter()
                .filter_map(|(i, r)| st.target_of(i).map(|t| (t, r)))
                .collect();
            st.dialog = Some(Dialog::Tree { kill, excluded, opened: false });
        }
        Cmd::Suspend(id) => {
            if let Some(t) = st.target_of(id) {
                st.dialog = Some(Dialog::Confirm { suspend: true, targets: vec![t], opened: false });
            }
        }
        Cmd::Resume(id) => {
            if let Some(target) = st.target_of(id) {
                st.job(Job::Resume { target });
            }
        }
        Cmd::Priority(id, priority) => {
            if let Some(target) = st.target_of(id) {
                st.job(Job::SetPriority { target, priority });
            }
        }
        Cmd::Efficiency(id, on) => {
            if let Some(target) = st.target_of(id) {
                st.job(Job::SetEfficiency { target, on });
            }
        }
        Cmd::Affinity(id) => {
            if let Some(target) = st.target_of(id) {
                st.job(Job::GetAffinity { target: target.clone() });
                st.dialog = Some(Dialog::Affinity { target, state: crate::app::Load::Loading, mask: 0, opened: false });
            }
        }
        Cmd::Front(id) => {
            if let Some(target) = st.target_of(id) {
                st.job(Job::BringToFront { target });
            }
        }
        Cmd::OpenLocation(path) => st.job(Job::OpenLocation { path }),
        Cmd::Properties(path) => st.job(Job::ShowProperties { path }),
        Cmd::Copy(ids) => {
            let text = copy_text(st, &ids);
            ui.ctx().copy_text(text);
        }
        Cmd::OpenTaskmgr => st.job(Job::OpenTaskManager),
    }
}

fn toolbar(st: &mut State, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let edit = egui::TextEdit::singleline(&mut st.procs.filter)
            .hint_text("名前・PID・パスで絞り込み(Ctrl+F)")
            .desired_width(280.0);
        let resp = ui.add(edit);
        if std::mem::take(&mut st.procs.focus_filter) {
            resp.request_focus();
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            st.procs.filter.clear();
        }
        let l = ui.label("範囲");
        egui::ComboBox::from_id_salt("scope")
            .selected_text(st.procs.scope.label())
            .show_ui(ui, |ui| {
                for s in Scope::ALL {
                    ui.selectable_value(&mut st.procs.scope, s, s.label());
                }
            })
            .response
            .labelled_by(l.id);
        ui.add_space(8.0);
        let mut view = st.cfg.view;
        ui.selectable_value(&mut view, View::Tree, "ツリー");
        ui.selectable_value(&mut view, View::Flat, "一覧");
        if view != st.cfg.view {
            st.cfg.view = view;
            st.save_config();
        }
        ui.add_space(8.0);
        ui.menu_button("列…", |ui| columns_menu(st, ui));
    });
}

/// FR-3: 表示・順序。名前の列は外せない
fn columns_menu(st: &mut State, ui: &mut Ui) {
    ui.set_max_width(260.0);
    let vis = st.cfg.columns.visible_columns();
    let n = vis.len();
    // (外す位置, 入れ替える 2 つ, 足す列) のどれか 1 つ
    let mut remove = None;
    let mut swap = None;
    let mut add = None;
    for (i, c) in vis.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut on = true;
            let r = ui.add_enabled(*c != Column::Name, egui::Checkbox::new(&mut on, column_title(st, *c)));
            if *c == Column::Name {
                r.on_disabled_hover_text("名前の列は外せません");
            }
            if !on {
                remove = Some(i);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(i + 1 < n, egui::Button::new("▼").small()).on_hover_text("後ろへ").clicked() {
                    swap = Some((i, i + 1));
                }
                if ui.add_enabled(i > 0, egui::Button::new("▲").small()).on_hover_text("前へ").clicked() {
                    swap = Some((i, i - 1));
                }
            });
        });
    }
    ui.separator();
    for c in Column::ALL.into_iter().filter(|c| !vis.contains(c)) {
        let mut on = false;
        if ui.checkbox(&mut on, column_title(st, c)).changed() && on {
            add = Some(c);
        }
    }
    let mut out = vis.clone();
    if let Some(i) = remove {
        out.remove(i);
    }
    if let Some((a, b)) = swap {
        out.swap(a, b);
    }
    if let Some(c) = add {
        out.push(c);
    }
    if out != vis {
        st.cfg.columns.set_visible_columns(&out);
        st.save_config();
        st.procs.invalidate();
    }
}

/// 既定のアイコン(触れない・パスの無い行。FR-8)
fn default_icon(ui: &mut Ui, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    let r = rect.shrink(2.0);
    ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, pal.weak), egui::StrokeKind::Inside);
    ui.painter().line_segment([r.left_top() + egui::vec2(0.0, 3.5), r.right_top() + egui::vec2(0.0, 3.5)], egui::Stroke::new(1.0, pal.weak));
}

/// 開閉の印。字に頼らず三角を描く
fn toggle(ui: &mut Ui, expanded: bool, color: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    let c = rect.center();
    let pts = if expanded {
        vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
    } else {
        vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    resp.clicked()
}

pub fn state_color(pal: &Palette, s: ProcState) -> Color32 {
    match s {
        ProcState::Running | ProcState::Exiting => pal.weak,
        ProcState::Suspended => pal.warn,
        ProcState::NotResponding => pal.danger,
        ProcState::TouchFree => pal.accent,
    }
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    toolbar(st, ui);
    let pal = theme::palette(ui);
    if st.diag.spi_fail_in_row >= 4 {
        egui::Frame::new().fill(pal.band_danger).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("プロセスの一覧を取得できません").color(pal.danger));
        });
    }
    // 詳細パネル(表の下。既定は窓の 35%。§4.4)
    let full_h = ui.available_height();
    if st.procs.detail.collapsed {
        egui::Panel::bottom("detail_c").resizable(false).show(ui, |ui| details::ui(st, ui));
    } else {
        egui::Panel::bottom("detail")
            .resizable(true)
            .default_size(full_h * 0.35)
            .min_size(120.0)
            .show(ui, |ui| details::ui(st, ui));
    }
    if st.snap.seq == 0 {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| ui.label("プロセスを読み込んでいます"));
        return;
    }
    jump_if_asked(st);
    refresh(st);
    keyboard(st, ui);
    refresh(st);
    if st.procs.visible.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| ui.label("一致するプロセスはありません"));
        return;
    }
    // 届いたアイコンをテクスチャにする
    for (path, slot) in st.icons.iter_mut() {
        if let IconSlot::Rgba(rgba) = slot {
            let img = egui::ColorImage::from_rgba_unmultiplied([16, 16], rgba);
            *slot = IconSlot::Tex(ui.ctx().load_texture(format!("icon:{path}"), img, egui::TextureOptions::LINEAR));
        }
    }
    let scroll_row = st.procs.scroll_to.take().and_then(|id| st.procs.visible.iter().position(|v| st.snap.rows[v.index].id == id));
    // 列の境目の線が詳細パネルの上まで伸びないよう、表の場所で切る
    let rect = ui.available_rect_before_wrap();
    let ev = ui
        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            // 列が窓の幅を超えたら横に送る
            egui::ScrollArea::horizontal().id_salt("procs_h").auto_shrink([false, false]).show(ui, |ui| table(st, ui, &pal, scroll_row)).inner
        })
        .inner;
    handle(st, ui, ev);
}

/// FR-44・S8: 祖先を開き、絞り込みを外してから選ぶ
fn jump_if_asked(st: &mut State) {
    let Some(id) = st.procs.scroll_to else { return };
    if st.snap.find(id).is_none() {
        st.procs.scroll_to = None;
        return;
    }
    st.procs.filter.clear();
    st.procs.scope = Scope::All;
    let mut cur = st.snap.find(id).and_then(|r| r.parent);
    let mut guard = 0;
    while let Some(p) = cur {
        st.procs.collapsed.remove(&p);
        cur = st.snap.find(p).and_then(|r| r.parent);
        guard += 1;
        if guard > 256 {
            break;
        }
    }
    st.procs.invalidate();
    st.procs.select_only(id);
}

fn table(st: &State, ui: &mut Ui, pal: &Palette, scroll_row: Option<usize>) -> Events {
    let mut ev = Events::default();
    let cols = st.cfg.columns.visible_columns();
    let sort_col = st.cfg.columns.sort_column();
    let desc = st.cfg.columns.sort.descending;
    let snap = st.snap.clone();
    let visible = &st.procs.visible;
    let tree_view = st.cfg.view == View::Tree;
    let table_clip = ui.clip_rect();
    let spacing_x = ui.spacing().item_spacing.x;
    let mut tb = TableBuilder::new(ui)
        // egui_extras は列の幅を位置で覚えるので、並びが変わったら表を作り直して config の幅を当て直す
        .id_salt(("procs", &cols))
        .striped(false)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);
    for c in &cols {
        tb = tb.column(egui_extras::Column::initial(st.cfg.columns.width(*c)).range(40.0..=800.0).clip(true));
    }
    if let Some(i) = scroll_row {
        tb = tb.scroll_to_row(i, Some(Align::Center));
    }
    let show_sig = cols.contains(&Column::Signature);
    tb.header(ROW_H + 2.0, |mut h| {
        for c in &cols {
            h.col(|ui| {
                let mark = if *c == sort_col { if desc { " ▼" } else { " ▲" } } else { "" };
                let text = RichText::new(format!("{}{mark}", column_title(st, *c))).strong();
                let layout = if c.is_numeric() { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
                ui.with_layout(layout, |ui| {
                    if ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false)).clicked() {
                        ev.sort = Some(*c);
                    }
                });
            });
        }
    })
    .body(|body| {
        let widths = body.widths().to_vec();
        let changed = cols.iter().zip(&widths).any(|(c, w)| (st.cfg.columns.width(*c) - w).abs() > 0.5);
        let gap = spacing_x;
        let total_w = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
        if changed {
            ev.widths = Some(widths);
        }
        body.rows(ROW_H, visible.len(), |mut row| {
            let v = &visible[row.index()];
            let r = &snap.rows[v.index];
            let selected = st.procs.selected.contains(&r.id);
            row.set_selected(selected);
            let bg = if r.is_gone > 0 {
                Some(pal.gone_row)
            } else if r.is_new > 0 {
                Some(pal.new_row)
            } else {
                None
            };
            if let Some(p) = r.path() {
                if !st.icons.contains_key(p) {
                    ev.icons.push(p.to_string());
                }
                if show_sig && !st.sigs.contains_key(p) {
                    ev.sigs.push(p.to_string());
                }
            }
            for (ci, c) in cols.iter().enumerate() {
                row.col(|ui| {
                    // 新しい行・終わった行の背景は行の幅いっぱいに塗る(セルは切り抜かれるので層に直接描く)
                    if let (Some(bg), false, 0) = (bg, selected, ci) {
                        let r = ui.max_rect();
                        let row_rect = egui::Rect::from_min_max(
                            egui::pos2(r.left() - gap / 2.0, r.center().y - ROW_H / 2.0),
                            egui::pos2(r.left() + total_w + gap / 2.0, r.center().y + ROW_H / 2.0),
                        );
                        let clip = egui::Rect::from_x_y_ranges(table_clip.x_range(), ui.clip_rect().y_range());
                        ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(clip).rect_filled(row_rect, 0.0, bg);
                    }
                    let text_color = if v.dimmed { pal.weak } else { ui.visuals().text_color() };
                    match c {
                        Column::Name => {
                            if tree_view {
                                ui.add_space(v.depth as f32 * 16.0);
                                if v.has_children {
                                    if toggle(ui, v.expanded, pal.weak) {
                                        ev.toggle = Some(r.id);
                                    }
                                } else {
                                    ui.add_space(16.0);
                                }
                            }
                            match r.path().and_then(|p| st.icons.get(p)) {
                                Some(IconSlot::Tex(t)) => {
                                    ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(16.0))));
                                }
                                _ => default_icon(ui, pal),
                            }
                            ui.add(egui::Label::new(RichText::new(display_name(st, r)).color(text_color)).truncate().selectable(false));
                        }
                        Column::State => {
                            ui.add(egui::Label::new(RichText::new(r.state.label()).color(state_color(pal, r.state))).selectable(false));
                        }
                        c if c.is_numeric() => {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(cell_text(st, r, *c)).color(text_color)).selectable(false));
                            });
                        }
                        _ => {
                            ui.add(egui::Label::new(RichText::new(cell_text(st, r, *c)).color(text_color)).truncate().selectable(false));
                        }
                    }
                });
            }
            let resp = row.response();
            if resp.double_clicked() {
                ev.double = Some(r.id);
            } else if resp.clicked() {
                ev.click = Some((r.id, ui_modifiers(&resp)));
            }
            if resp.secondary_clicked() {
                ev.secondary = Some(r.id);
            }
            let ids = if selected { st.procs.selected.clone() } else { vec![r.id] };
            resp.context_menu(|ui| {
                ui.set_min_width(220.0);
                context_menu(st, ui, r, &ids, &mut ev.cmds);
                if !ev.cmds.is_empty() {
                    ui.close();
                }
            });
        });
    });
    ev
}

fn ui_modifiers(r: &egui::Response) -> Modifiers {
    r.ctx.input(|i| i.modifiers)
}

fn handle(st: &mut State, ui: &Ui, ev: Events) {
    if let Some(ws) = ev.widths {
        for (c, w) in st.cfg.columns.visible_columns().iter().zip(ws) {
            st.cfg.columns.widths.insert(c.key().to_string(), w.clamp(40.0, 800.0));
        }
        st.save_config();
    }
    if let Some(c) = ev.sort {
        let s = &mut st.cfg.columns.sort;
        if s.column == c.key() {
            s.descending = !s.descending;
        } else {
            s.column = c.key().into();
            // 数字は大きい順から(AC-5)
            s.descending = c.is_numeric() || c == Column::Cpu;
        }
        st.save_config();
    }
    if let Some(id) = ev.toggle {
        if !st.procs.collapsed.remove(&id) {
            st.procs.collapsed.insert(id);
        }
        st.procs.invalidate();
    }
    if let Some((id, m)) = ev.click {
        click(st, id, m);
    }
    if let Some(id) = ev.secondary
        && !st.procs.selected.contains(&id)
    {
        st.procs.select_only(id);
    }
    if let Some(id) = ev.double {
        st.procs.select_only(id);
        st.procs.detail.collapsed = false;
    }
    for p in ev.icons {
        st.icons.insert(p.clone(), IconSlot::Pending);
        st.job(Job::Icon { path: p });
    }
    for p in ev.sigs {
        st.sigs.insert(p.clone(), None);
        st.job(Job::Signature { path: p });
    }
    for c in ev.cmds {
        run_cmd(st, ui, c);
    }
}

fn click(st: &mut State, id: ProcId, m: Modifiers) {
    if m.command {
        if let Some(i) = st.procs.selected.iter().position(|x| *x == id) {
            st.procs.selected.remove(i);
        } else {
            st.procs.selected.push(id);
        }
        st.procs.anchor = Some(id);
        if let Some(one) = st.procs.single() {
            st.procs.detail.reset(one);
        }
    } else if m.shift
        && let Some(a) = st.procs.anchor
    {
        let pos = |x: ProcId| st.procs.visible.iter().position(|v| st.snap.rows[v.index].id == x);
        if let (Some(i), Some(j)) = (pos(a), pos(id)) {
            let (lo, hi) = (i.min(j), i.max(j));
            st.procs.selected = st.procs.visible[lo..=hi].iter().map(|v| st.snap.rows[v.index].id).collect();
        }
    } else {
        st.procs.select_only(id);
    }
}

/// 画面設計書 §8 の S1 のキー。文字の欄に入力しているときと確認の窓の間は見ない
fn keyboard(st: &mut State, ui: &mut Ui) {
    if st.dialog.is_some() || ui.ctx().egui_wants_keyboard_input() || st.settings.open || st.locks.open {
        return;
    }
    let pressed = |ui: &mut Ui, k: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if pressed(ui, Key::Delete) && !st.procs.selected.is_empty() {
        let ids = st.procs.selected.clone();
        ask_terminate(st, &ids);
        return;
    }
    let copy = ui.input_mut(|i| {
        let ev = i.events.iter().position(|e| matches!(e, egui::Event::Copy));
        if let Some(p) = ev {
            i.events.remove(p);
            true
        } else {
            i.consume_key(Modifiers::COMMAND, Key::C)
        }
    });
    if copy && !st.procs.selected.is_empty() {
        let ids = st.procs.selected.clone();
        let text = copy_text(st, &ids);
        ui.ctx().copy_text(text);
        log::info("copy", &format!("rows={}", ids.len()));
    }
    if pressed(ui, Key::Enter) && st.procs.single().is_some() {
        st.procs.detail.collapsed = false;
    }
    let n = st.procs.visible.len();
    if n == 0 {
        return;
    }
    let cur = st
        .procs
        .single()
        .or(st.procs.anchor)
        .and_then(|id| st.procs.visible.iter().position(|v| st.snap.rows[v.index].id == id));
    let page = 20;
    let mut target = None;
    if pressed(ui, Key::ArrowDown) {
        target = Some(cur.map_or(0, |c| (c + 1).min(n - 1)));
    }
    if pressed(ui, Key::ArrowUp) {
        target = Some(cur.map_or(0, |c| c.saturating_sub(1)));
    }
    if pressed(ui, Key::PageDown) {
        target = Some(cur.map_or(0, |c| (c + page).min(n - 1)));
    }
    if pressed(ui, Key::PageUp) {
        target = Some(cur.map_or(0, |c| c.saturating_sub(page)));
    }
    if pressed(ui, Key::Home) {
        target = Some(0);
    }
    if pressed(ui, Key::End) {
        target = Some(n - 1);
    }
    if let Some(t) = target {
        let id = st.snap.rows[st.procs.visible[t].index].id;
        st.procs.select_only(id);
        st.procs.scroll_to = Some(id);
    }
    // ← → でツリーを畳む・開く
    if st.cfg.view == View::Tree
        && let Some(c) = cur
    {
        let v = st.procs.visible[c].clone();
        let id = st.snap.rows[v.index].id;
        if pressed(ui, Key::ArrowLeft) && v.has_children && v.expanded {
            st.procs.collapsed.insert(id);
            st.procs.invalidate();
        }
        if pressed(ui, Key::ArrowRight) && v.has_children && !v.expanded {
            st.procs.collapsed.remove(&id);
            st.procs.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_has_header_and_rows_tab_separated() {
        let st = crate::demo::state();
        let id = st.snap.rows.iter().find(|r| r.name == "notepad.exe").unwrap().id;
        let text = copy_text(&st, &[id]);
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("名前\tPID\t状態\tCPU"));
        assert!(lines[1].starts_with("notepad.exe\t"));
    }

    #[test]
    fn self_row_and_parent_gone() {
        let st = crate::demo::state();
        let me = st.snap.find_pid(st.env.self_pid).unwrap();
        assert_eq!(display_name(&st, me), "ProcSight.exe(ProcSight)");
        let orphan = st.snap.rows.iter().find(|r| r.name == "orphan.exe").unwrap();
        assert!(cell_text(&st, orphan, Column::ParentPid).ends_with("(終了)"));
        // 中核・自分は終了を灰色
        let csrss = st.snap.rows.iter().find(|r| r.name == "csrss.exe").unwrap();
        assert!(terminate_refusal(&st, csrss).is_some());
        assert!(terminate_refusal(&st, me).is_some());
    }

    #[test]
    fn filter_and_tree_counts() {
        let mut st = crate::demo::state();
        refresh(&mut st);
        let all = st.procs.visible.len();
        assert!(all > 20);
        st.procs.filter = "TESTEE".into();
        refresh(&mut st);
        // 根 1・子 3・孫 9(AC-3 の形)
        assert_eq!(st.procs.visible.iter().filter(|v| !v.dimmed).count(), 13);
        st.cfg.view = View::Flat;
        refresh(&mut st);
        assert_eq!(st.procs.visible.len(), 13);
    }
}
