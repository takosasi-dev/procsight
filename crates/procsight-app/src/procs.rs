//! S1 プロセス(画面設計書 §4): 絞り込み・範囲・ツリーと一覧・列・並べ替え・色・アイコン・右クリックのメニュー・キーボード。
//! 表は egui_extras で見える行だけを描く(D-1・NFR-4)。並びは Snapshot・絞り込み・並べ替え・畳みが変わったときだけ作り直す。
//! 終了・一時停止は確認の窓(D1・D2)を開くだけで、Job は積まない(INV-5)。

use eframe::egui;
use crate::app::{IconSlot, State};
use crate::dialogs::Dialog;
use crate::theme::{self, Palette, size, space};
use crate::{details, log, widgets};
use egui::{Align, Align2, Color32, FontId, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, Ui, Vec2};
use egui_extras::TableBuilder;
use procsight_core::config::{Config, DoubleClick, TreeStart, View};
use procsight_core::filter::{self, Query, Scope, SortValue};
use procsight_core::format;
use procsight_core::rules;
use procsight_core::tree::{self, Forest, VisibleRow};
use procsight_core::ring::Ring;
use procsight_core::{Column, Priority, ProcId, ProcState, ProcessRow, Snapshot};
use procsight_win::jobs::{Job, Target};
use std::collections::{HashMap, HashSet};

/// 見出しの高さ(上に全体の値、下に列の名前の 2 段)
const HEADER_H: f32 = 42.0;

/// 見出しの上の段に出す全体の値(Snapshot が変わったときだけ作る)
#[derive(Default)]
struct HeaderTotals {
    seq: u64,
    cpu: Option<f32>,
    mem: Option<f32>,
    gpu: Option<f32>,
    io_read: Option<f64>,
    io_write: Option<f64>,
}

impl HeaderTotals {
    fn of(snap: &Snapshot) -> HeaderTotals {
        let sum = |f: fn(&ProcessRow) -> Option<f64>| {
            let mut any = false;
            let s: f64 = snap.rows.iter().filter_map(f).inspect(|_| any = true).sum();
            any.then_some(s)
        };
        HeaderTotals {
            seq: snap.seq,
            cpu: snap.perf.cpu_total,
            mem: snap.perf.mem.as_ref().and_then(|m| m.used_pct()),
            gpu: snap.perf.gpus.as_ref().and_then(|g| g.iter().map(|x| x.util_pct).reduce(f32::max)),
            io_read: sum(|r| r.io_read_bps),
            io_write: sum(|r| r.io_write_bps),
        }
    }

    /// (上の段の文字, 塗りの割合)
    fn of_column(&self, c: Column) -> Option<(String, f32)> {
        let pct = |v: f32| (format!("{v:.0}%"), v / 100.0);
        match c {
            Column::Cpu => self.cpu.map(pct),
            Column::Mem => self.mem.map(pct),
            Column::Gpu => self.gpu.map(pct),
            Column::IoRead => self.io_read.map(|v| (format::speed(Some(v)), 0.0)),
            Column::IoWrite => self.io_write.map(|v| (format::speed(Some(v)), 0.0)),
            _ => None,
        }
    }
}

/// 使用量の塗りの割合(0〜1)。CPU と GPU は平方根で小さな値も見えるように、メモリは物理メモリの 20% で最大、
/// I/O は 16 KB/s〜50 MB/s の対数(常に少し読み書きしている行まで塗らない)
fn heat_frac(mem_total: Option<u64>, r: &ProcessRow, c: Column) -> Option<f32> {
    const IO_LO: f64 = 16.0 * 1024.0;
    const IO_HI: f64 = 50.0 * 1024.0 * 1024.0;
    let io = |b: Option<f64>| b.map(|b| ((b / IO_LO).max(1.0).log10() / (IO_HI / IO_LO).log10()) as f32);
    match c {
        Column::Cpu => r.cpu_pct.map(|v| (v / 100.0).max(0.0).sqrt()),
        Column::Gpu => r.gpu_pct.map(|v| (v / 100.0).max(0.0).sqrt()),
        Column::Mem => r.mem.zip(mem_total.filter(|t| *t > 0)).map(|(m, t)| m as f32 / t as f32 * 5.0),
        Column::IoRead => io(r.io_read_bps),
        Column::IoWrite => io(r.io_write_bps),
        _ => None,
    }
}

/// 使用量の塗りを rect に。range は設定の(塗り始める所, 一番濃くする所)。既定(0.02・1.0)なら widgets::paint_heat と同じ色
fn paint_heat(ui: &Ui, rect: Rect, pal: &Palette, range: (f32, f32), frac: f32) {
    if let Some(c) = theme::heat_range(pal, frac, range.0, range.1) {
        ui.painter().rect_filled(rect, 0.0, c);
    }
}

/// CPU の推移の縦の位置(0〜1)。目盛りは全部の行で 0〜100% に固定し、使用量の塗り(heat_frac)と同じ平方根で曲げる。
/// 行ごとの最大に合わせると 0.5% の揺れと 90% の山が同じ高さに見え、行どうしを見比べられない。
/// 直線の 0〜100% だと数 % の行が底に張り付く。平方根なら 1% が 1 割、25% が半分の高さで、両方を満たす
fn spark_y(pct: f32) -> f32 {
    (pct / 100.0).clamp(0.0, 1.0).sqrt()
}

/// 折れ線の色。今の CPU が SPARK_HOT 未満はグラフの色、以上は使用量の塗り(heat)と同じ系統の橙。
/// 間を混ぜると青と橙の間の濁った灰色になるので、2 色で切り替える
fn spark_color(pal: &Palette, now_pct: f32) -> Color32 {
    if now_pct >= SPARK_HOT { pal.graph2 } else { pal.graph }
}

/// 折れ線を橙にする今の CPU(%)。CPU の列の塗りがはっきり橙に見え始める辺り
const SPARK_HOT: f32 = 15.0;

/// 小さな折れ線と、その下の薄い塗り(左が古く、右が今。points 回に満たない間は右に寄せる)。y は値 → 縦の位置(0〜1)。
/// perf.rs の paint_line と同じ形。points は設定のグラフの長さ。詳細パネルのグラフも使う
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_spark(
    painter: &egui::Painter,
    rect: Rect,
    vals: impl ExactSizeIterator<Item = f32>,
    points: usize,
    y: impl Fn(f32) -> f32,
    color: Color32,
    fill: Color32,
    width: f32,
) {
    let points = points.max(2);
    let total = vals.len();
    let len = total.min(points);
    if len < 2 || rect.width() < 4.0 {
        return;
    }
    let n = (points - 1) as f32;
    let start = points - len;
    // 輪の方が長い(長さを変えた直後)ときは新しい方の points 回分
    let vals = vals.skip(total - len);
    let mut pts = Vec::with_capacity(len);
    // 塗りは 1 枚の網で(区切りごとの多角形だと、境目に細い線が見える。中がへこんだ形なので凸の多角形は使えない)
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(len * 2);
    mesh.reserve_triangles((len - 1) * 2);
    for (i, v) in vals.enumerate() {
        let x = rect.left() + (start + i) as f32 / n * rect.width();
        let p = egui::pos2(x, rect.bottom() - y(v) * rect.height());
        pts.push(p);
        mesh.colored_vertex(p, fill);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), fill);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
}

/// CPU の推移の列のセル。マウスを置くと今・最大・平均を出す
fn cpu_spark_cell(st: &State, ui: &mut Ui, pal: &Palette, r: &ProcessRow, row_rect: Rect, selected: bool, weak: bool) {
    let Some(h) = st.procs.cpu_hist.get(&r.id) else { return };
    let cell = ui.max_rect();
    let rect = Rect::from_x_y_ranges(cell.left() + space::S..=cell.right() - space::S, row_rect.top() + 3.0..=row_rect.bottom() - 3.0);
    let now = h.vals.last().copied().unwrap_or(0.0);
    let max = h.vals.iter().copied().fold(0.0, f32::max);
    // 選んだ行は地が強調の色なので、線は濃い文字の色に。絞り込みで残した親は弱い色
    let color = if selected {
        ui.visuals().strong_text_color()
    } else if weak {
        pal.weak
    } else {
        spark_color(pal, now)
    };
    let fill = widgets::tint(color, if selected { 56 } else { 44 });
    paint_spark(ui.painter(), rect, h.vals.iter().copied(), st.procs.points, spark_y, color, fill, 1.25);
    let resp = ui.interact(rect, ui.scope_id().with(("cpu_spark", r.id)), Sense::hover());
    resp.on_hover_ui(|ui| {
        let n = h.vals.len().max(1);
        let avg = h.vals.iter().sum::<f32>() / n as f32;
        // 数字だけ設定の数え方で(線の形と色は掛ける前の値)
        let k = cpu_factor(st);
        ui.label(RichText::new(format!("CPU の推移(直近 {} 回)", h.vals.len())).strong());
        ui.label(format!("今 {}  最大 {}  平均 {}", format::pct(Some(now * k)), format::pct(Some(max * k)), format::pct(Some(avg * k))));
    });
}

#[derive(Clone, Debug, PartialEq)]
struct ListKey {
    seq: u64,
    filter: String,
    scope: Scope,
    col: Column,
    desc: bool,
    tree: bool,
    /// CPU の数え方(絞り込みの cpu>N を表の数字と比べる)
    cpu_factor: f32,
    generation: u64,
}

/// プロセスごとの CPU の数字に掛ける数(設定の「CPU の数え方」。全体の CPU には掛けない)
pub fn cpu_factor(st: &State) -> f32 {
    st.cfg.cpu_factor(st.snap.logical_cpus)
}

pub struct ProcsView {
    pub filter: String,
    pub scope: Scope,
    pub focus_filter: bool,
    pub selected: Vec<ProcId>,
    anchor: Option<ProcId>,
    pub collapsed: HashSet<ProcId>,
    /// 設定「ツリーで新しく見えたプロセス: 全部畳む」で畳んだ物(v0.4.0)。絞り込みの間は見ない(一致した子が隠れるため)
    pub auto_collapsed: HashSet<ProcId>,
    key: Option<ListKey>,
    generation: u64,
    pub visible: Vec<VisibleRow>,
    forest: Forest,
    /// FR-44・S8 から: この行を選んで見える位置まで動かす
    pub scroll_to: Option<ProcId>,
    pub detail: details::DetailState,
    /// 解析した絞り込み(元の文字と対。文字が変わったときだけ解析し直す)
    query: (String, Query),
    totals: HeaderTotals,
    /// CPU の推移の列(v0.3.0): 識別子 → 直近 60 回の CPU %。メモリの中だけに持ち、保存しない
    cpu_hist: HashMap<ProcId, CpuHist>,
    /// 見出しの上の段に描く全体の CPU の推移
    cpu_total_hist: Ring<f32>,
    /// 最後に推移を進めた Snapshot の seq
    hist_seq: u64,
    /// 設定の写し(on_snapshot には cfg が来ないので、refresh・ui で合わせる)。グラフの長さ
    points: usize,
    /// 設定の写し。ツリーで初めて見えたプロセスを畳むか
    tree_start: TreeStart,
}

/// 1 つのプロセスの CPU の推移
struct CpuHist {
    /// 最後に一覧で見かけた Snapshot の seq(見かけなくなった物を消す印)
    seen: u64,
    vals: Ring<f32>,
}

impl ProcsView {
    /// ツリーで畳んで描く識別子。絞り込みの間(no_filter が false)は「全部畳む」で畳んだ物を除く
    fn folded_set(&self, no_filter: bool) -> HashSet<ProcId> {
        if no_filter && !self.auto_collapsed.is_empty() { self.collapsed.union(&self.auto_collapsed).copied().collect() } else { self.collapsed.clone() }
    }

    /// 今その行が畳まれて見えているか(folded_set と同じ決まり)
    pub fn is_folded(&self, id: ProcId) -> bool {
        self.collapsed.contains(&id) || (self.filter.trim().is_empty() && self.auto_collapsed.contains(&id))
    }

    /// 畳む・開く。開くときは「全部畳む」で畳んだ印も外す
    pub fn set_folded(&mut self, id: ProcId, fold: bool) {
        if fold {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
            self.auto_collapsed.remove(&id);
        }
    }

    pub fn new(cfg: &Config) -> ProcsView {
        let mut v = ProcsView {
            query: Default::default(),
            totals: HeaderTotals::default(),
            cpu_hist: HashMap::new(),
            cpu_total_hist: Ring::default(),
            hist_seq: 0,
            points: 0,
            tree_start: cfg.tree_start,
            filter: String::new(),
            scope: Scope::All,
            focus_filter: false,
            selected: Vec::new(),
            anchor: None,
            collapsed: HashSet::new(),
            auto_collapsed: HashSet::new(),
            key: None,
            generation: 0,
            visible: Vec::new(),
            forest: Forest::default(),
            scroll_to: None,
            detail: details::DetailState::default(),
        };
        v.follow_config(cfg);
        v
    }

    /// 設定の写しを合わせる。グラフの長さが変わったら、推移と詳細のグラフの輪を同じ長さにする
    /// (減らすと古い方を捨てる)。設定の画面は値を変えるだけで知らせないので、描く度に呼ぶ
    pub fn follow_config(&mut self, cfg: &Config) {
        self.tree_start = cfg.tree_start;
        let n = cfg.graph_points;
        if self.points == n {
            return;
        }
        self.points = n;
        for h in self.cpu_hist.values_mut() {
            h.vals.set_capacity(n);
        }
        self.cpu_total_hist.set_capacity(n);
        self.detail.cpu.set_capacity(n);
        self.detail.mem.set_capacity(n);
    }

    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    pub fn on_snapshot(&mut self, snap: &Snapshot) {
        self.detail.push_history(snap);
        self.push_cpu_history(snap);
    }

    /// CPU の推移を 1 回分進める。seq が変わったときだけ。終わった(is_gone)・一覧から消えた識別子は消す。
    /// 最小化中の周(rows_stale)は行の値が無いので、足しも消しもしない
    fn push_cpu_history(&mut self, snap: &Snapshot) {
        if snap.seq == self.hist_seq {
            return;
        }
        self.hist_seq = snap.seq;
        if let Some(c) = snap.perf.cpu_total {
            self.cpu_total_hist.push(c);
        }
        if snap.rows_stale {
            return;
        }
        let (points, fold_new) = (self.points, self.tree_start == TreeStart::Collapsed);
        for r in snap.rows.iter().filter(|r| r.is_gone == 0) {
            // 推移の鍵は「見たことのある識別子」も兼ねる。初めて見えた物は、設定が「畳む」ならツリーで畳む
            // (既に見た物・設定を変える前からある物は触らない)
            let h = self.cpu_hist.entry(r.id).or_insert_with(|| {
                if fold_new {
                    self.auto_collapsed.insert(r.id);
                }
                CpuHist { seen: 0, vals: Ring::new(points) }
            });
            h.seen = snap.seq;
            // 触れないプロセスも SPI の値なので足す。値が無い周(最初の 1 回など)は足さない
            if let Some(c) = r.cpu_pct.filter(|c| c.is_finite()) {
                h.vals.push(c.max(0.0));
            }
        }
        self.cpu_hist.retain(|_, h| h.seen == snap.seq);
        // 一覧から消えた識別子は畳みの印も消す(畳むが既定だと増え続けるため)
        if !self.collapsed.is_empty() || !self.auto_collapsed.is_empty() {
            let ids: HashSet<ProcId> = snap.rows.iter().map(|r| r.id).collect();
            self.collapsed.retain(|id| ids.contains(id));
            self.auto_collapsed.retain(|id| ids.contains(id));
        }
    }

    /// CPU の推移(古い順)。テストと CSV の確かめに使う
    pub fn cpu_history(&self, id: ProcId) -> Option<impl ExactSizeIterator<Item = f32> + '_> {
        self.cpu_hist.get(&id).map(|h| h.vals.iter().copied())
    }

    /// 推移を持っているプロセスの数
    pub fn cpu_history_len(&self) -> usize {
        self.cpu_hist.len()
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
        Column::Cpu => cpu_text(st, r),
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
        // コピーと CSV では今の CPU を出す(表では cpu_spark_cell が折れ線を描く)
        Column::CpuHistory => cpu_text(st, r),
    }
}

/// プロセスの CPU の文字(設定の数え方を掛ける。表・コピー・CSV で同じ)
fn cpu_text(st: &State, r: &ProcessRow) -> String {
    format::pct(r.cpu_pct.map(|v| v * cpu_factor(st)))
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

/// 並びを必要なときだけ作り直す(CSV の保存も、書く前に呼んで今の Snapshot に合わせる)
pub(crate) fn refresh(st: &mut State) {
    st.procs.follow_config(&st.cfg);
    let key = ListKey {
        seq: st.snap.seq,
        filter: st.procs.filter.clone(),
        scope: st.procs.scope,
        col: st.cfg.columns.sort_column(),
        desc: st.cfg.columns.sort.descending,
        tree: st.cfg.view == View::Tree,
        cpu_factor: cpu_factor(st),
        generation: st.procs.generation,
    };
    if st.procs.key.as_ref() == Some(&key) {
        return;
    }
    if st.procs.query.0 != key.filter {
        st.procs.query = (key.filter.clone(), Query::parse(&key.filter));
    }
    if st.procs.totals.seq != st.snap.seq {
        st.procs.totals = HeaderTotals::of(&st.snap);
    }
    let snap = st.snap.clone();
    let rows = &snap.rows;
    let me = st.env.user.clone();
    let (scope, col, desc, k) = (key.scope, key.col, key.desc, key.cpu_factor);
    let st_ro: &State = st;
    let q = &st_ro.procs.query.1;
    let m = |r: &ProcessRow| filter::in_scope(r, scope, me.as_deref()) && q.matches_scaled(r, k);
    // System Idle Process(PID 0)は、どの列・どちら向きでも一番下(CPU の降順で上に居座るため)
    let cmp = |a: &ProcessRow, b: &ProcessRow| {
        (a.id.pid == 0).cmp(&(b.id.pid == 0)).then_with(|| {
            filter::compare_values(&sort_value(st_ro, a, col), &sort_value(st_ro, b, col), desc, a.id.pid, b.id.pid)
        })
    };
    let (forest, visible) = if key.tree {
        let f = tree::build(rows);
        // 自分で畳んだ物は v0.3.0 と同じにいつも見る。「全部畳む」で畳んだ物は、絞り込みの間は見ない
        // (全部の根が畳まれていると、一致した子が 1 行も出ないため)
        let folded = st_ro.procs.folded_set(key.filter.trim().is_empty());
        let v = tree::flatten_tree(rows, &f, &folded, &m, &cmp);
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

/// 「?」の印。マウスを置くと絞り込みの書き方の一覧を出す。
fn query_help(ui: &mut Ui, pal: &Palette) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
    let color = if resp.hovered() { pal.accent } else { pal.weak };
    ui.painter().circle_stroke(rect.center(), 8.0, Stroke::new(1.2, color));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "?", FontId::proportional(size::SMALL), color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "絞り込みの書き方"));
    resp.on_hover_ui(|ui| {
        ui.set_max_width(440.0);
        ui.label(RichText::new("絞り込みの書き方").strong());
        ui.label(RichText::new("空白で区切った語を全部満たす行を残します").color(pal.weak).size(size::SMALL));
        ui.add_space(space::S);
        egui::Grid::new("query_help").num_columns(2).spacing([space::L, space::S]).show(ui, |ui| {
            for (k, v) in filter::QUERY_HELP {
                ui.label(RichText::new(k).monospace().color(pal.accent));
                ui.label(v);
                ui.end_row();
            }
        });
    });
}

/// ツリーと一覧の 2 つの切り替え(枠の中に並べる)。
fn view_switch(st: &mut State, ui: &mut Ui, pal: &Palette) {
    let mut view = st.cfg.view;
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(6)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            for (v, label) in [(View::Tree, "ツリー"), (View::Flat, "一覧")] {
                let on = view == v;
                let fg = if on { ui.visuals().strong_text_color() } else { pal.weak };
                let b = egui::Button::new(RichText::new(label).color(fg))
                    .fill(if on { pal.selected } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(4)
                    .min_size(egui::vec2(52.0, 22.0))
                    .selected(on);
                if ui.add(b).clicked() {
                    view = v;
                }
            }
        });
    if view != st.cfg.view {
        st.cfg.view = view;
        st.save_config();
    }
}

fn toolbar(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    ui.add_space(space::S);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::M;
        ui.spacing_mut().interact_size.y = 26.0;
        let w = (ui.available_width() * 0.34).clamp(180.0, 380.0);
        let resp = widgets::search_box(ui, &pal, &mut st.procs.filter, "名前・PID・パスで絞り込み(Ctrl+F)", w);
        if std::mem::take(&mut st.procs.focus_filter) {
            resp.request_focus();
        }
        query_help(ui, &pal);
        ui.add_space(space::S);
        let l = ui.label(RichText::new("範囲").color(pal.weak));
        egui::ComboBox::from_id_salt("scope")
            .selected_text(st.procs.scope.label())
            .show_ui(ui, |ui| {
                for s in Scope::ALL {
                    ui.selectable_value(&mut st.procs.scope, s, s.label());
                }
            })
            .response
            .labelled_by(l.id);
        view_switch(st, ui, &pal);
        ui.menu_button("列…", |ui| columns_menu(st, ui));
    });
    ui.add_space(space::S);
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

/// 行のアイコン(届いていれば exe のアイコン、無ければ既定の印)。詳細パネルの見出しでも使う
pub fn row_icon(st: &State, ui: &mut Ui, r: &ProcessRow, pal: &Palette) {
    match r.path().and_then(|p| st.icons.get(p)) {
        Some(IconSlot::Tex(t)) => {
            ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(16.0))));
        }
        _ => default_icon(ui, pal),
    }
}

/// 状態のセル: 色の点と文字。触れないは札にして目立たせる
fn state_cell(ui: &mut Ui, pal: &Palette, s: ProcState) {
    if s == ProcState::TouchFree {
        widgets::pill(ui, s.label(), pal.accent);
        return;
    }
    let dot = match s {
        ProcState::Running => widgets::tint(pal.ok, 170),
        _ => state_color(pal, s),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, dot);
    ui.add(egui::Label::new(RichText::new(s.label()).color(state_color(pal, s))).selectable(false));
}

/// 名前の文字。切れているときだけ、マウスを置くと全部の名前とパスを出す
fn name_label(ui: &mut Ui, pal: &Palette, text: String, color: Color32, path: Option<&str>) {
    let (pos, galley, resp) = egui::Label::new(RichText::new(text).color(color)).truncate().selectable(false).layout_in_ui(ui);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, galley.text()));
    let elided = galley.elided;
    if ui.is_rect_visible(resp.rect) {
        ui.painter().galley(pos, galley.clone(), color);
    }
    if elided {
        resp.on_hover_ui(|ui| {
            ui.set_max_width(560.0);
            ui.label(RichText::new(galley.job.text.as_str()).strong());
            if let Some(p) = path {
                ui.label(RichText::new(p).color(pal.weak));
            }
        });
    }
}

/// 右寄せの列(数字の列と CPU の推移)。見出しの名前を右に寄せ、初めて押したときは大きい順に並べる
fn right_aligned(c: Column) -> bool {
    c.is_numeric() || c == Column::CpuHistory
}

/// 見出しの 1 つのセル。上に全体の値(塗り)、下に列の名前と並べ替えの印。押すと並べ替え。
/// `spark` があれば(CPU の推移の列)、上の段に全体の CPU の推移を描く
#[allow(clippy::too_many_arguments)]
fn header_cell(
    ui: &mut Ui,
    pal: &Palette,
    title: &str,
    c: Column,
    sort: Option<bool>,
    total: Option<(String, f32)>,
    heat: Option<(f32, f32)>,
    spark: Option<(&Ring<f32>, usize)>,
) -> bool {
    let rect = ui.max_rect();
    let resp = ui.interact(rect, ui.scope_id().with(("hdr", c.key())), Sense::click());
    if let (Some(range), Some((_, f))) = (heat, &total) {
        paint_heat(ui, rect, pal, range, *f);
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, widgets::tint(ui.visuals().strong_text_color(), 14));
    }
    let numeric = right_aligned(c);
    let pad = space::S;
    let (x, anchor_t) = if numeric { (rect.right() - pad, Align2::RIGHT_TOP) } else { (rect.left() + pad, Align2::LEFT_TOP) };
    let p = ui.painter();
    if let Some((t, _)) = &total {
        p.text(egui::pos2(x, rect.top() + 3.0), anchor_t, t, FontId::proportional(size::BODY + 2.0), ui.visuals().strong_text_color());
    }
    if let Some((ring, points)) = spark {
        // 隣の CPU の列の大きな数字と同じ高さの帯に、全体の CPU(行と同じ目盛り)
        let band = Rect::from_x_y_ranges(rect.left() + pad..=rect.right() - pad, rect.top() + 4.0..=rect.top() + 19.0);
        let latest = ring.last().copied().unwrap_or(0.0);
        let color = spark_color(pal, latest);
        paint_spark(p, band, ring.iter().copied(), points, spark_y, color, widgets::tint(color, 40), 1.25);
    }
    let title_color = if sort.is_some() { pal.accent } else { pal.weak };
    let bottom = rect.bottom() - 4.0;
    // 並べ替えの印は字に頼らず三角を描く(数字の列は名前の左、他は右)
    let font = FontId::proportional(size::SMALL + 1.0);
    let g = p.layout_no_wrap(title.to_string(), font, title_color);
    let tw = g.size().x;
    let tri_w = if sort.is_some() { 12.0 } else { 0.0 };
    let tx = if numeric { x - tw } else { x };
    p.galley(egui::pos2(tx, bottom - g.size().y), g, title_color);
    if let Some(desc) = sort {
        let cx = if numeric { tx - tri_w / 2.0 - 1.0 } else { tx + tw + tri_w / 2.0 + 1.0 };
        let cy = bottom - 7.0;
        let pts = if desc {
            vec![egui::pos2(cx - 4.0, cy - 2.0), egui::pos2(cx + 4.0, cy - 2.0), egui::pos2(cx, cy + 3.0)]
        } else {
            vec![egui::pos2(cx - 4.0, cy + 3.0), egui::pos2(cx + 4.0, cy + 3.0), egui::pos2(cx, cy - 2.0)]
        };
        p.add(egui::Shape::convex_polygon(pts, pal.accent, Stroke::NONE));
    }
    let mark = match sort {
        Some(true) => " ▼",
        Some(false) => " ▲",
        None => "",
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{title}{mark}")));
    resp.clicked()
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
    st.procs.follow_config(&st.cfg);
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
    // 表の場所がとても低いとき(小さな窓で大きな文字)は、詳細を見出しの行だけにして表を残す
    let forced = !st.procs.detail.collapsed && full_h < 260.0;
    if st.procs.detail.collapsed || forced {
        // 畳んだ形で描く間だけ畳んだことにし、利用者の選び(collapsed)は変えない
        st.procs.detail.collapsed = true;
        egui::Panel::bottom("detail_c").resizable(false).show(ui, |ui| details::ui(st, ui));
        if forced && st.procs.detail.collapsed {
            st.procs.detail.collapsed = false;
        }
    } else {
        egui::Panel::bottom("detail")
            .resizable(true)
            .default_size(full_h * 0.35)
            .min_size(120.0)
            .show(ui, |ui| details::ui(st, ui));
    }
    if st.snap.seq == 0 {
        widgets::empty_state(ui, &pal, "プロセスを読み込んでいます", None);
        return;
    }
    jump_if_asked(st);
    refresh(st);
    keyboard(st, ui);
    refresh(st);
    if st.procs.visible.is_empty() {
        let hint = (!st.procs.filter.trim().is_empty()).then_some("欄の右の × で絞り込みを消せます。書き方は欄の横の「?」にあります");
        widgets::empty_state(ui, &pal, "一致するプロセスはありません", hint);
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
        st.procs.set_folded(p, false);
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
    let row_h = st.cfg.density.row_h();
    let heat_on = st.cfg.heat_map;
    let heat_range = heat_on.then(|| (st.cfg.heat_start.frac(), st.cfg.heat_full.frac()));
    let mem_total = snap.perf.mem.as_ref().map(|m| m.total);
    let totals = &st.procs.totals;
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
    // 余りの幅を取る空の列。選んだ行・新しい行の色と見出しの線を窓の右端まで届かせる
    tb = tb.column(egui_extras::Column::remainder().at_least(0.0).resizable(false).clip(true));
    if let Some(i) = scroll_row {
        tb = tb.scroll_to_row(i, Some(Align::Center));
    }
    let show_sig = cols.contains(&Column::Signature);
    tb.header(HEADER_H, |mut h| {
        for (ci, c) in cols.iter().enumerate() {
            h.col(|ui| {
                // 見出しの下の線は窓の右端まで(セルは切り抜かれるので層に直接描く)
                if ci == 0 {
                    let r = ui.max_rect();
                    let clip = Rect::from_x_y_ranges(table_clip.x_range(), r.top()..=r.bottom() + 2.0);
                    ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(clip).hline(
                        r.left() - spacing_x..=table_clip.right(),
                        r.bottom() + 1.0,
                        Stroke::new(1.0, pal.border),
                    );
                }
                let sort = (*c == sort_col).then_some(desc);
                let spark = (*c == Column::CpuHistory).then_some((&st.procs.cpu_total_hist, st.procs.points));
                if header_cell(ui, pal, column_title(st, *c), *c, sort, totals.of_column(*c), heat_range, spark) {
                    ev.sort = Some(*c);
                }
            });
        }
        h.col(|_| {});
    })
    .body(|body| {
        let widths = body.widths()[..cols.len()].to_vec();
        let changed = cols.iter().zip(&widths).any(|(c, w)| (st.cfg.columns.width(*c) - w).abs() > 0.5);
        let gap = spacing_x;
        if changed {
            ev.widths = Some(widths);
        }
        body.rows(row_h, visible.len(), |mut row| {
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
            // 塗りは選んだ行・新しい行・終わった行・絞り込みで残した親・System Idle Process では使わない(行の色を優先)
            let heat_row = heat_on && !selected && bg.is_none() && !v.dimmed && r.id.pid != 0;
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
                    let cell = ui.max_rect();
                    let row_rect = Rect::from_x_y_ranges(cell.x_range(), cell.center().y - row_h / 2.0..=cell.center().y + row_h / 2.0);
                    if ci == 0 {
                        let clip = Rect::from_x_y_ranges(table_clip.x_range(), ui.clip_rect().y_range());
                        let layer = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(clip);
                        // 新しい行・終わった行の背景は行の幅いっぱいに塗る(セルは切り抜かれるので層に直接描く)
                        if let (Some(bg), false) = (bg, selected) {
                            let full = Rect::from_x_y_ranges(row_rect.left() - gap / 2.0..=table_clip.right(), row_rect.y_range());
                            layer.rect_filled(full, 0.0, bg);
                        }
                        // 触れないプロセスは左の端に強調の色の線
                        if r.touch_free {
                            let bar = Rect::from_x_y_ranges(row_rect.left()..=row_rect.left() + 3.0, row_rect.y_range());
                            layer.rect_filled(bar, 0.0, pal.accent);
                        }
                    }
                    if heat_row
                        && let (Some(range), Some(f)) = (heat_range, heat_frac(mem_total, r, *c))
                    {
                        paint_heat(ui, row_rect, pal, range, f);
                    }
                    let text_color = if v.dimmed { pal.weak } else { ui.visuals().text_color() };
                    match c {
                        Column::Name => {
                            ui.add_space(space::S);
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
                            row_icon(st, ui, r, pal);
                            name_label(ui, pal, display_name(st, r), text_color, r.path());
                        }
                        Column::State => {
                            ui.add_space(space::S);
                            state_cell(ui, pal, r.state);
                        }
                        // System Idle Process は塗りと同じく使用量の色にしない(空きの割合なので)
                        Column::CpuHistory => cpu_spark_cell(st, ui, pal, r, row_rect, selected, v.dimmed || r.id.pid == 0),
                        c if c.is_numeric() => {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(space::S);
                                ui.add(egui::Label::new(RichText::new(cell_text(st, r, *c)).color(text_color)).selectable(false));
                            });
                        }
                        _ => {
                            ui.add_space(space::S);
                            ui.add(egui::Label::new(RichText::new(cell_text(st, r, *c)).color(text_color)).truncate().selectable(false));
                        }
                    }
                });
            }
            row.col(|_| {});
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
            // 数字は大きい順から(AC-5)。CPU の推移は今の CPU で並べる
            s.descending = right_aligned(c);
        }
        st.save_config();
    }
    if let Some(id) = ev.toggle {
        let folded = st.procs.is_folded(id);
        st.procs.set_folded(id, !folded);
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
        double_click(st, id);
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

/// 行のダブルクリック(設定の「行のダブルクリック」)。どれでも行は選ぶ。
/// 詳細: 詳細パネルを開く。ファイルの場所: メニューと同じ条件(触れない・パスが無いなら何もしない)で同じ Job
fn double_click(st: &mut State, id: ProcId) {
    st.procs.select_only(id);
    match st.cfg.double_click {
        DoubleClick::Details => st.procs.detail.collapsed = false,
        DoubleClick::OpenLocation => {
            let path = st.snap.find(id).filter(|r| touch_refusal(r).is_none()).and_then(|r| r.path()).map(str::to_string);
            if let Some(path) = path {
                st.job(Job::OpenLocation { path });
            }
        }
        DoubleClick::Nothing => {}
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
            st.procs.set_folded(id, true);
            st.procs.invalidate();
        }
        if pressed(ui, Key::ArrowRight) && v.has_children && !v.expanded {
            st.procs.set_folded(id, false);
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
    fn spark_scale_is_shared_by_all_rows_and_bends_like_heat() {
        assert_eq!(spark_y(0.0), 0.0);
        assert!((spark_y(1.0) - 0.1).abs() < 1e-6);
        assert!((spark_y(25.0) - 0.5).abs() < 1e-6);
        assert_eq!(spark_y(250.0), 1.0);
        assert_eq!(spark_y(-3.0), 0.0);
        // 低い間はグラフの色、高いと heat と同じ橙
        let p = crate::theme::DARK;
        assert_eq!(spark_color(&p, 1.0), p.graph);
        assert_eq!(spark_color(&p, 14.9), p.graph);
        assert_eq!(spark_color(&p, 15.0), p.graph2);
        assert_eq!(spark_color(&p, 60.0), p.graph2);
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
