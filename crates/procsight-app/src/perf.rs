//! S2 パフォーマンス(画面設計書 §5): 左にページの並び(名前・今の値・小さなグラフ)、右に見出し・大きな値・自前で描くグラフ・数字の格子・上位のプロセス。
//! 履歴は Snapshot ごとに直近 60 回分だけメモリに持つ(非スコープ: 保存)。更新間隔を変えたら空から始める。上位のプロセスも Snapshot ごとに 1 回だけ並べる。
//! 取れないページは並びに残し、右に「この PC では取得できません」を出す(E-11)。

use crate::app::{State, Tab};
use crate::theme::{self, Palette, size, space};
use crate::widgets;
use eframe::egui;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use procsight_core::format;
use procsight_core::ring::{GRAPH_POINTS, Ring};
use procsight_core::{PerfSample, ProcId, ProcessRow, Snapshot};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Cpu,
    Memory,
    Disk(String),
    Net(String),
    Gpu(String),
    /// 値が 1 つも無い種類(E-11)
    NoDisk,
    NoNet,
    NoGpu,
}

#[derive(Default)]
pub struct Series {
    pub a: Ring<f32>,
    pub b: Ring<f32>,
    pub c: Ring<f32>,
}

/// 「このリソースを使っているプロセス」の 1 行
#[derive(Clone, Debug, PartialEq)]
pub struct TopRow {
    pub id: ProcId,
    pub name: String,
    pub value: f64,
}

/// 資源ごとの上位 5 個(Snapshot ごとに並べ直す。描くたびには並べない)
#[derive(Clone, Debug, Default)]
pub struct Tops {
    pub cpu: Vec<TopRow>,
    pub mem: Vec<TopRow>,
    /// 読み取り+書き込み(B/s。全ディスクの合計)
    pub io: Vec<TopRow>,
    pub gpu: Vec<TopRow>,
}

const TOP_N: usize = 5;

impl Tops {
    pub fn from_rows(rows: &[ProcessRow]) -> Tops {
        Tops {
            cpu: top(rows, |r| r.cpu_pct.map(f64::from)),
            mem: top(rows, |r| r.mem.map(|v| v as f64)),
            io: top(rows, |r| match (r.io_read_bps, r.io_write_bps) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
            }),
            gpu: top(rows, |r| r.gpu_pct.map(f64::from)),
        }
    }
}

/// 値の大きい順に 5 個。System Idle Process(PID 0)・終わった行・0 の行は除く
fn top(rows: &[ProcessRow], value: impl Fn(&ProcessRow) -> Option<f64>) -> Vec<TopRow> {
    let mut v: Vec<(f64, &ProcessRow)> = rows
        .iter()
        .filter(|r| r.id.pid != 0 && r.is_gone == 0)
        .filter_map(|r| value(r).filter(|x| x.is_finite() && *x > 0.0).map(|x| (x, r)))
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.pid.cmp(&b.1.id.pid)));
    v.into_iter().take(TOP_N).map(|(value, r)| TopRow { id: r.id, name: r.name.clone(), value }).collect()
}

#[derive(Default)]
pub struct PerfView {
    pub page: Page,
    pub per_core: bool,
    pub cpu: Ring<f32>,
    pub cores: Vec<Ring<f32>>,
    /// 使用中のバイト
    pub mem: Ring<f32>,
    /// 名前ごと: ディスクは (アクティブ%, 読み取り, 書き込み)、ネットは (送信, 受信)、GPU は (使用率)
    pub disks: BTreeMap<String, Series>,
    pub nets: BTreeMap<String, Series>,
    pub gpus: BTreeMap<String, Series>,
    pub last: PerfSample,
    pub tops: Tops,
}

impl PerfView {
    /// 更新間隔が変わったとき: グラフは空から。選んでいるページ・切り替え・最後の値と上位のプロセスは残す
    pub fn clear(&mut self) {
        let page = std::mem::take(&mut self.page);
        let per_core = self.per_core;
        let last = std::mem::take(&mut self.last);
        let tops = std::mem::take(&mut self.tops);
        *self = PerfView { page, per_core, last, tops, ..Default::default() };
    }

    pub fn push(&mut self, s: &Snapshot) {
        let p = &s.perf;
        self.cpu.push(p.cpu_total.unwrap_or(0.0));
        if self.cores.len() != p.cpu_per_core.len() {
            self.cores = (0..p.cpu_per_core.len()).map(|_| Ring::default()).collect();
        }
        for (r, v) in self.cores.iter_mut().zip(&p.cpu_per_core) {
            r.push(*v);
        }
        if let Some(m) = &p.mem {
            self.mem.push(m.in_use() as f32);
        }
        for d in p.disks.iter().flatten() {
            let e = self.disks.entry(d.name.clone()).or_default();
            e.a.push(d.active_pct);
            e.b.push(d.read_bps as f32);
            e.c.push(d.write_bps as f32);
        }
        for n in p.nets.iter().flatten() {
            let e = self.nets.entry(n.name.clone()).or_default();
            e.a.push(n.send_bps as f32);
            e.b.push(n.recv_bps as f32);
        }
        for g in p.gpus.iter().flatten() {
            let e = self.gpus.entry(g.luid.clone()).or_default();
            e.a.push(g.util_pct);
        }
        self.last = p.clone();
        if !s.rows_stale {
            self.tops = Tops::from_rows(&s.rows);
        }
    }
}

// ---------------------------------------------------------------- 小さな絵

/// 線と塗りを rect に描く(左が古く、右が今。60 回に満たない間は右に寄せる)
fn paint_line(painter: &egui::Painter, rect: Rect, vals: &Ring<f32>, max: f32, color: Color32, fill_alpha: u8, width: f32) {
    if vals.len() < 2 || max <= 0.0 {
        return;
    }
    let n = GRAPH_POINTS as f32 - 1.0;
    let start = GRAPH_POINTS - vals.len().min(GRAPH_POINTS);
    let pts: Vec<Pos2> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + (start + i) as f32 / n * rect.width();
            let f = if v.is_finite() { (v / max).clamp(0.0, 1.0) } else { 0.0 };
            egui::pos2(x, rect.bottom() - f * rect.height())
        })
        .collect();
    // 塗りは 1 枚の網で(区切りごとの多角形だと、境目に細い線が見える)
    let fill = widgets::tint(color, fill_alpha);
    let mut mesh = egui::Mesh::default();
    for (i, p) in pts.iter().enumerate() {
        mesh.colored_vertex(*p, fill);
        mesh.colored_vertex(egui::pos2(p.x, rect.bottom()), fill);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
}

/// 小さな線のグラフ(枠つき)。`max` を上端にする(詳細パネルも使う)
pub fn sparkline(ui: &mut Ui, values: impl Iterator<Item = f32>, max: f32, size: Vec2, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let pal = theme::palette(ui);
    let mut ring = Ring::default();
    for v in values {
        ring.push(v);
    }
    paint_spark(ui, rect, &ring, max, color, &pal);
}

fn paint_spark(ui: &Ui, rect: Rect, ring: &Ring<f32>, max: f32, color: Color32, pal: &Palette) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, pal.surface);
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    paint_line(&painter, rect.shrink2(egui::vec2(1.0, 2.0)), ring, max, color, 48, 1.5);
}

fn ring_max(r: &Ring<f32>) -> f32 {
    r.iter().copied().filter(|v| v.is_finite()).fold(0.0, f32::max)
}

/// 1 行に収まらない文字は「…」で切る
fn galley_trunc(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    ui.painter().layout_job(job)
}

// ---------------------------------------------------------------- 縦軸の単位

#[derive(Clone, Copy)]
enum Unit {
    Pct,
    Bytes,
    Speed,
}

const UNIT_NAMES_BYTES: [&str; 4] = ["B", "KB", "MB", "GB"];
const UNIT_NAMES_SPEED: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];

/// max の大きさから 1024 進の段を選ぶ(0〜3)
fn unit_step(max: f64) -> usize {
    let mut k = 0;
    let mut m = max;
    while m >= 1024.0 && k < 3 {
        m /= 1024.0;
        k += 1;
    }
    k
}

impl Unit {
    /// 縦軸の目盛りの文字。0 は「0」だけ、ほかは max と同じ単位で
    fn axis(self, v: f64, max: f64) -> String {
        if v <= 0.0 {
            return "0".into();
        }
        match self {
            Unit::Pct => format!("{v:.0}%"),
            Unit::Bytes | Unit::Speed => {
                let k = unit_step(max);
                let x = v / 1024f64.powi(k as i32);
                let name = if matches!(self, Unit::Bytes) { UNIT_NAMES_BYTES[k] } else { UNIT_NAMES_SPEED[k] };
                if (x - x.round()).abs() < 0.005 {
                    format!("{x:.0} {name}")
                } else if (x * 10.0 - (x * 10.0).round()).abs() < 0.05 {
                    format!("{x:.1} {name}")
                } else {
                    format!("{x:.2} {name}")
                }
            }
        }
    }

    /// 指したところの値
    fn value(self, v: f64) -> String {
        match self {
            Unit::Pct => format!("{v:.1}%"),
            Unit::Bytes => format::bytes(v.max(0.0) as u64),
            Unit::Speed => format::speed(Some(v)),
        }
    }
}

/// 速さの縦軸の上端: 見えている最大を、同じ単位で 1・2・4・6・8・10 の切りのよい数に上げる(最小 1 KB/s)
fn nice_speed_max(raw: f64) -> f64 {
    // 線が上の縁に張り付かないよう 1 割の余白を足す
    let raw = (raw * 1.1).max(1024.0);
    let k = unit_step(raw);
    let u = 1024f64.powi(k as i32);
    let x = raw / u;
    let p = 10f64.powf(x.log10().floor());
    let m = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0].into_iter().map(|c| c * p).find(|c| *c >= x - 1e-9).unwrap_or(10.0 * p);
    // 1000 台の切りのよい数が 1024 を超えたら、1 つ上の単位の 1 にする(「1.95 MB/s」のような半端な目盛りを避ける)
    if m >= 1024.0 { 1024.0 * u } else { m * u }
}

// ---------------------------------------------------------------- 大きいグラフ

struct Line<'a> {
    name: &'a str,
    ring: &'a Ring<f32>,
    color: Color32,
}

/// 大きいグラフ。横の線は 25% ごと、縦の線は 15 回ごとに薄く。縦軸の左に単位つきの目盛り。指すとその時の値
fn graph(ui: &mut Ui, pal: &Palette, height: f32, max: f64, unit: Unit, lines: &[Line<'_>]) {
    let small = FontId::proportional(size::SMALL);
    let ticks = [0.0, 0.25, 0.5, 0.75, 1.0];
    let labels: Vec<Arc<egui::Galley>> = ticks.iter().map(|t| ui.painter().layout_no_wrap(unit.axis(max * t, max), small.clone(), pal.weak)).collect();
    let gutter = labels.iter().map(|g| g.size().x).fold(0.0, f32::max) + space::M;
    let text_h = labels[0].size().y;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let plot = Rect::from_min_max(egui::pos2(rect.left() + gutter, rect.top() + text_h / 2.0), egui::pos2(rect.right(), rect.bottom() - text_h / 2.0));
    let painter = ui.painter();
    painter.rect_filled(plot, CornerRadius::same(4), pal.surface);
    let grid = Stroke::new(1.0, widgets::tint(pal.border, 200));
    for (t, g) in ticks.iter().zip(&labels) {
        let y = plot.bottom() - plot.height() * *t as f32;
        if *t > 0.0 && *t < 1.0 {
            painter.hline(plot.x_range(), y, grid);
        }
        painter.galley(egui::pos2(plot.left() - space::M - g.size().x, y - g.size().y / 2.0), g.clone(), pal.weak);
    }
    let thin = Stroke::new(1.0, widgets::tint(pal.border, 110));
    for k in 1..4 {
        let x = plot.left() + plot.width() * k as f32 / 4.0;
        painter.vline(x, plot.y_range(), thin);
    }
    painter.rect_stroke(plot, CornerRadius::same(4), Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let inner = plot.shrink2(egui::vec2(1.0, 1.0));
    let clip = ui.painter_at(plot);
    let alpha = if lines.len() > 1 { 34 } else { 52 };
    for l in lines {
        paint_line(&clip, inner, l.ring, max as f32, l.color, alpha, 2.0);
    }
    // 指したところ: 縦の線と点、値の札
    if let Some(pos) = resp.hover_pos().filter(|p| plot.contains(*p)) {
        let n = GRAPH_POINTS - 1;
        let idx = (((pos.x - inner.left()) / inner.width()) * n as f32).round().clamp(0.0, n as f32) as usize;
        let x = inner.left() + idx as f32 / n as f32 * inner.width();
        clip.vline(x, plot.y_range(), Stroke::new(1.0, pal.weak));
        let mut rows = Vec::new();
        for l in lines {
            let start = GRAPH_POINTS - l.ring.len().min(GRAPH_POINTS);
            if idx >= start
                && let Some(v) = l.ring.iter().nth(idx - start)
            {
                let f = if max > 0.0 { (*v as f64 / max).clamp(0.0, 1.0) as f32 } else { 0.0 };
                clip.circle_filled(egui::pos2(x, inner.bottom() - f * inner.height()), 3.5, l.color);
                rows.push((l.name, l.color, unit.value(*v as f64)));
            }
        }
        let ago = n - idx;
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(if ago == 0 { "今".to_string() } else { format!("{ago} 回前") }).color(pal.weak).size(size::SMALL));
            if rows.is_empty() {
                ui.label(RichText::new(format::DASH).color(pal.weak));
            }
            for (name, color, v) in rows {
                ui.horizontal(|ui| {
                    widgets::dot_label(ui, name, color);
                    ui.label(RichText::new(v).strong());
                });
            }
        });
    }
}

/// グラフの上の行: 左に何のグラフか、右に凡例(2 本のとき)
fn graph_caption(ui: &mut Ui, pal: &Palette, text: &str, legend: &[(&str, Color32)]) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).color(pal.weak).size(size::SMALL));
        for (name, color) in legend {
            ui.add_space(space::M);
            widgets::dot_label(ui, name, *color);
        }
    });
}

/// グラフの下の行: 横軸が何回分か
fn graph_footer(ui: &mut Ui, pal: &Palette, interval_ms: u64) {
    let secs = interval_ms * (GRAPH_POINTS as u64) / 1000;
    let span = if secs >= 60 && secs.is_multiple_of(60) { format!("{} 分", secs / 60) } else { format!("{secs} 秒") };
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{GRAPH_POINTS} 回分(直近 {span})")).color(pal.weak).size(size::SMALL));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new("今").color(pal.weak).size(size::SMALL)));
    });
}

// ---------------------------------------------------------------- 見出しと数字

/// 大きな題と、右に装置の名前(長ければ「…」で切る)
fn header(ui: &mut Ui, pal: &Palette, title: &str, device: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(size::TITLE + 4.0).strong());
        if !device.is_empty() {
            ui.add_space(space::L);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(device).color(pal.weak)).truncate());
            });
        }
    });
}

/// 今の値を大きく。右に弱い文字の説明
fn hero(ui: &mut Ui, pal: &Palette, value: &str, sub: &str) {
    ui.horizontal(|ui| {
        let big = ui.label(RichText::new(value).size(size::HERO).strong()).rect.height();
        ui.add_space(space::S);
        // 説明は大きな値の下の線にそろえる
        ui.vertical(|ui| {
            ui.add_space((big - size::BODY * 1.6).max(0.0));
            ui.add(egui::Label::new(RichText::new(sub).color(pal.weak)).truncate());
        });
    });
    ui.add_space(space::S);
}

/// widgets::stat の格子。幅に合わせて列の数を決め、狭ければ折り返す
fn stat_grid(ui: &mut Ui, pal: &Palette, items: &[(&str, String)]) {
    if items.is_empty() {
        return;
    }
    let w = ui.available_width();
    let cols = ((w / 132.0).floor() as usize).clamp(1, items.len());
    let cell = w / cols as f32;
    for chunk in items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (k, v) in chunk {
                ui.allocate_ui_with_layout(egui::vec2(cell, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(cell - space::M);
                    widgets::stat(ui, pal, k, v.clone());
                });
            }
        });
        ui.add_space(space::M);
    }
}

/// 下の段: 広ければ左右に、狭ければ縦に並べる
fn two_up<R>(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui) -> R) -> R {
    if ui.available_width() >= 760.0 {
        let mut out = None;
        ui.columns(2, |c| {
            left(&mut c[0]);
            out = Some(right(&mut c[1]));
        });
        out.expect("columns が右を呼ばない")
    } else {
        left(ui);
        ui.add_space(space::M);
        right(ui)
    }
}

fn card_full<R>(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    widgets::card(ui, pal, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
    .inner
}

// ---------------------------------------------------------------- 上位のプロセス

struct TopSpec<'a> {
    title: &'a str,
    col: &'a str,
    rows: &'a [TopRow],
    unit: Unit,
    /// 横棒の上端(None なら一番上の行の値)
    full: Option<f64>,
}

/// 上位 5 個の小さな表。行を押すと、その行の識別子を返す
fn top_table(ui: &mut Ui, pal: &Palette, spec: &TopSpec<'_>) -> Option<ProcId> {
    let mut picked = None;
    widgets::section(ui, pal, spec.title);
    let font = FontId::proportional(size::BODY);
    let small = FontId::proportional(size::SMALL);
    let row_h = 26.0;
    let (pid_w, bar_w, val_w) = (64.0, 72.0, 80.0);
    let layout = |rect: Rect| {
        let val = Rect::from_min_max(egui::pos2(rect.right() - val_w - space::M, rect.top()), egui::pos2(rect.right() - space::M, rect.bottom()));
        let bar = Rect::from_min_max(egui::pos2(val.left() - space::M - bar_w, rect.top()), egui::pos2(val.left() - space::M, rect.bottom()));
        let pid = Rect::from_min_max(egui::pos2(bar.left() - space::M - pid_w, rect.top()), egui::pos2(bar.left() - space::M, rect.bottom()));
        let name = Rect::from_min_max(egui::pos2(rect.left() + space::M, rect.top()), egui::pos2(pid.left() - space::M, rect.bottom()));
        (name, pid, bar, val)
    };
    // 列の名前
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
    let (n, p, _, v) = layout(rect);
    for (r, right, t) in [(n, false, "名前"), (p, true, "PID"), (v, true, spec.col)] {
        let anchor = if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
        let x = if right { r.right() } else { r.left() };
        ui.painter().text(egui::pos2(x, r.center().y), anchor, t, small.clone(), pal.weak);
    }
    if spec.rows.is_empty() {
        ui.add_space(space::S);
        ui.label(RichText::new("使っているプロセスはありません").color(pal.weak));
        return None;
    }
    let full = spec.full.unwrap_or_else(|| spec.rows.first().map_or(1.0, |r| r.value)).max(f64::MIN_POSITIVE);
    let text = ui.visuals().text_color();
    for row in spec.rows {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
        let label = format!("{}(PID {})", row.name, row.id.pid);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if ui.is_rect_visible(rect) {
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(4), widgets::tint(pal.accent, 34));
            }
            let (n, p, b, v) = layout(rect);
            let g = galley_trunc(ui, &row.name, font.clone(), text, n.width());
            ui.painter().galley(egui::pos2(n.left(), n.center().y - g.size().y / 2.0), g, text);
            ui.painter().text(egui::pos2(p.right(), p.center().y), Align2::RIGHT_CENTER, row.id.pid.to_string(), small.clone(), pal.weak);
            let track = Rect::from_center_size(b.center(), egui::vec2(b.width(), 6.0));
            ui.painter().rect_filled(track, CornerRadius::same(3), widgets::tint(pal.weak, 50));
            let f = (row.value / full).clamp(0.0, 1.0) as f32;
            if f > 0.0 {
                let mut fill = track;
                fill.set_width((track.width() * f).max(3.0));
                ui.painter().rect_filled(fill, CornerRadius::same(3), pal.graph);
            }
            ui.painter().text(egui::pos2(v.right(), v.center().y), Align2::RIGHT_CENTER, spec.unit.value(row.value), font.clone(), text);
        }
        if resp.clicked() {
            picked = Some(row.id);
        }
        resp.on_hover_text("押すとプロセスのタブでこの行を選びます");
    }
    picked
}

// ---------------------------------------------------------------- 左の並び

/// 左の並びの 1 項目。選んでいる物は強調の色の縦の棒。名前と今の値を左に、小さなグラフを右に
fn side_item(ui: &mut Ui, pal: &Palette, sel: bool, name: &str, value: &str, spark: Option<(&Ring<f32>, f32)>) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 54.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, sel, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if sel {
            painter.rect_filled(rect, CornerRadius::same(5), widgets::tint(pal.accent, 30));
            let bar = Rect::from_min_size(egui::pos2(rect.left(), rect.top() + 8.0), egui::vec2(3.0, rect.height() - 16.0));
            painter.rect_filled(bar, CornerRadius::same(2), pal.accent);
        } else if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(5), ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let spark_w = 64.0;
        let spark_rect = Rect::from_min_size(egui::pos2(rect.right() - spark_w - space::M, rect.center().y - 15.0), egui::vec2(spark_w, 30.0));
        let text_w = spark_rect.left() - rect.left() - space::L - space::S;
        let strong = if sel { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
        let g1 = galley_trunc(ui, name, FontId::proportional(size::BODY), strong, text_w);
        let g2 = galley_trunc(ui, value, FontId::proportional(size::SMALL), pal.weak, text_w);
        let top = rect.center().y - (g1.size().y + g2.size().y) / 2.0;
        let x = rect.left() + space::L;
        let h1 = g1.size().y;
        painter.galley(egui::pos2(x, top), g1, strong);
        painter.galley(egui::pos2(x, top + h1), g2, pal.weak);
        if let Some((r, max)) = spark {
            paint_spark(ui, spark_rect, r, max, pal.graph, pal);
        }
    }
    resp.clicked()
}

fn disk_label(name: &str) -> String {
    format!("ディスク {name}")
}

fn gpu_name(p: &PerfSample, luid: &str) -> String {
    p.gpus.iter().flatten().find(|g| g.luid == luid).map_or_else(|| luid.to_string(), |g| g.name.clone())
}

fn side(pv: &mut PerfView, ui: &mut Ui, pal: &Palette) {
    let p = &pv.last;
    let mut pick = None;
    ui.add_space(space::S);
    if side_item(ui, pal, pv.page == Page::Cpu, "CPU", &format::pct(p.cpu_total), Some((&pv.cpu, 100.0))) {
        pick = Some(Page::Cpu);
    }
    let mem_v = p.mem.as_ref().map_or_else(
        || format::DASH.to_string(),
        |m| format!("{} / {}({:.0}%)", gb1(m.in_use()), format::bytes(m.total), m.used_pct().unwrap_or(0.0)),
    );
    let mem_max = p.mem.as_ref().map_or(1.0, |m| m.total as f32);
    if side_item(ui, pal, pv.page == Page::Memory, "メモリ", &mem_v, Some((&pv.mem, mem_max))) {
        pick = Some(Page::Memory);
    }
    match &p.disks {
        Some(ds) if !ds.is_empty() => {
            for d in ds {
                let page = Page::Disk(d.name.clone());
                let ring = pv.disks.get(&d.name).map(|s| (&s.a, 100.0));
                if side_item(ui, pal, pv.page == page, &disk_label(&d.name), &format!("{:.0}%", d.active_pct), ring) {
                    pick = Some(page);
                }
            }
        }
        _ => {
            if side_item(ui, pal, pv.page == Page::NoDisk, "ディスク", format::DASH, None) {
                pick = Some(Page::NoDisk);
            }
        }
    }
    match &p.nets {
        Some(ns) if !ns.is_empty() => {
            for n in ns {
                let page = Page::Net(n.name.clone());
                let ring = pv.nets.get(&n.name).map(|s| (&s.b, ring_max(&s.b).max(ring_max(&s.a)).max(1.0)));
                let v = format!("↓{} ↑{}",format::speed(Some(n.recv_bps)), format::speed(Some(n.send_bps)));
                if side_item(ui, pal, pv.page == page, &n.name, &v, ring) {
                    pick = Some(page);
                }
            }
        }
        _ => {
            if side_item(ui, pal, pv.page == Page::NoNet, "ネットワーク", format::DASH, None) {
                pick = Some(Page::NoNet);
            }
        }
    }
    match &p.gpus {
        Some(gs) if !gs.is_empty() => {
            for g in gs {
                let page = Page::Gpu(g.luid.clone());
                let ring = pv.gpus.get(&g.luid).map(|s| (&s.a, 100.0));
                if side_item(ui, pal, pv.page == page, &g.name, &format!("{:.0}%", g.util_pct), ring) {
                    pick = Some(page);
                }
            }
        }
        _ => {
            if side_item(ui, pal, pv.page == Page::NoGpu, "GPU", format::DASH, None) {
                pick = Some(Page::NoGpu);
            }
        }
    }
    if let Some(pg) = pick {
        pv.page = pg;
    }
}

/// 左の並びの短い形(小数 1 桁の GB)
fn gb1(v: u64) -> String {
    format!("{:.1}", v as f64 / (1024.0 * 1024.0 * 1024.0))
}

// ---------------------------------------------------------------- 右のページ

const UNAVAILABLE: &str = "この PC では取得できません";

struct Ctx<'a> {
    pal: Palette,
    graph_h: f32,
    interval_ms: u64,
    totals: &'a procsight_core::Totals,
    env: &'a procsight_win::EnvInfo,
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    egui::Panel::left("perf_side").resizable(false).exact_size(236.0).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| side(&mut st.perf, ui, &pal));
    });
    let (pv, snap, env, interval_ms) = (&mut st.perf, &st.snap, &st.env, st.cfg.interval_ms);
    let mut jump = None;
    egui::CentralPanel::default_margins().show(ui, |ui| {
        // ScrollArea の中では高さに限りが無いので、グラフの高さは外で決める
        let graph_h = (ui.available_height() * 0.34).clamp(150.0, 300.0);
        let cx = Ctx { pal, graph_h, interval_ms, totals: &snap.totals, env };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            jump = page(ui, &cx, pv);
            ui.add_space(space::L);
        });
    });
    if let Some(id) = jump {
        st.procs.scroll_to = Some(id);
        st.tab = Tab::Processes;
    }
}

fn page(ui: &mut Ui, cx: &Ctx<'_>, pv: &mut PerfView) -> Option<ProcId> {
    let pal = &cx.pal;
    let p = pv.last.clone();
    // 更新間隔を変えた直後は履歴が空(グラフは空から伸びる。取れないとは出さない)
    let empty = Series::default();
    match pv.page.clone() {
        Page::Cpu => cpu_page(ui, cx, pv, &p),
        Page::Memory => {
            header(ui, pal, "メモリ", &p.mem.as_ref().map_or_else(String::new, |m| format!("{} 搭載", format::bytes(m.total))));
            let Some(m) = &p.mem else {
                widgets::empty_state(ui, pal, UNAVAILABLE, None);
                return None;
            };
            let used = m.used_pct().unwrap_or(0.0);
            hero(ui, pal, &format::bytes(m.in_use()), &format!("/ {} 使用中({used:.0}%)", format::bytes(m.total)));
            widgets::meter(ui, pal, used / 100.0, ui.available_width(), pal.graph);
            ui.add_space(space::L);
            graph_caption(ui, pal, "使用中のメモリ", &[]);
            graph(ui, pal, cx.graph_h, m.total as f64, Unit::Bytes, &[Line { name: "使用中", ring: &pv.mem, color: pal.graph }]);
            graph_footer(ui, pal, cx.interval_ms);
            ui.add_space(space::M);
            let mut items = vec![
                ("使用中", format::bytes(m.in_use())),
                ("利用可能", format::bytes(m.available)),
                ("コミット済み / 上限", format!("{} / {}", format::bytes(m.commit_total), format::bytes(m.commit_limit))),
            ];
            for (k, v) in [("キャッシュ済み", m.cached), ("ページ プール", m.paged_pool), ("非ページ プール", m.nonpaged_pool)] {
                if v > 0 {
                    items.push((k, format::bytes(v)));
                }
            }
            let spec = TopSpec { title: "メモリを使っているプロセス", col: "メモリ", rows: &pv.tops.mem, unit: Unit::Bytes, full: None };
            two_up(
                ui,
                |ui| {
                    card_full(ui, pal, |ui| {
                        widgets::section(ui, pal, "使用状況");
                        stat_grid(ui, pal, &items);
                    });
                },
                |ui| card_full(ui, pal, |ui| top_table(ui, pal, &spec)),
            )
        }
        Page::Disk(name) => {
            header(ui, pal, &disk_label(&name), "");
            let d = p.disks.iter().flatten().find(|d| d.name == name).cloned();
            let s = pv.disks.get(&name).unwrap_or(&empty);
            let Some(d) = d else {
                widgets::empty_state(ui, pal, UNAVAILABLE, None);
                return None;
            };
            hero(ui, pal, &format!("{:.0}%", d.active_pct), "アクティブな時間");
            let rw_max = nice_speed_max(ring_max(&s.b).max(ring_max(&s.c)) as f64);
            let h = cx.graph_h * 0.8;
            two_up(
                ui,
                |ui| {
                    graph_caption(ui, pal, "アクティブな時間", &[]);
                    graph(ui, pal, h, 100.0, Unit::Pct, &[Line { name: "アクティブな時間", ring: &s.a, color: pal.graph }]);
                    graph_footer(ui, pal, cx.interval_ms);
                },
                |ui| {
                    graph_caption(ui, pal, "読み取り・書き込みの速さ", &[("読み取り", pal.graph), ("書き込み", pal.graph2)]);
                    graph(ui, pal, h, rw_max, Unit::Speed, &[
                        Line { name: "読み取り", ring: &s.b, color: pal.graph },
                        Line { name: "書き込み", ring: &s.c, color: pal.graph2 },
                    ]);
                    graph_footer(ui, pal, cx.interval_ms);
                },
            );
            ui.add_space(space::M);
            let items = [
                ("アクティブな時間", format!("{:.0}%", d.active_pct)),
                ("読み取りの速さ", format::speed(Some(d.read_bps))),
                ("書き込みの速さ", format::speed(Some(d.write_bps))),
                ("読み取りの最大", format::speed(Some(ring_max(&s.b) as f64))),
                ("書き込みの最大", format::speed(Some(ring_max(&s.c) as f64))),
            ];
            let spec = TopSpec { title: "I/O の多いプロセス(読み取り+書き込み)", col: "I/O", rows: &pv.tops.io, unit: Unit::Speed, full: None };
            two_up(
                ui,
                |ui| {
                    card_full(ui, pal, |ui| {
                        widgets::section(ui, pal, "使用状況");
                        stat_grid(ui, pal, &items);
                    });
                },
                |ui| card_full(ui, pal, |ui| top_table(ui, pal, &spec)),
            )
        }
        Page::Net(name) => {
            header(ui, pal, "ネットワーク", &name);
            let n = p.nets.iter().flatten().find(|n| n.name == name).cloned();
            let s = pv.nets.get(&name).unwrap_or(&empty);
            let Some(n) = n else {
                widgets::empty_state(ui, pal, UNAVAILABLE, None);
                return None;
            };
            hero(ui, pal, &format::speed(Some(n.recv_bps)), &format!("受信  ・  送信 {}", format::speed(Some(n.send_bps))));
            let max = nice_speed_max(ring_max(&s.a).max(ring_max(&s.b)) as f64);
            graph_caption(ui, pal, "送信と受信の速さ", &[("受信", pal.graph), ("送信", pal.graph2)]);
            graph(ui, pal, cx.graph_h, max, Unit::Speed, &[
                Line { name: "受信", ring: &s.b, color: pal.graph },
                Line { name: "送信", ring: &s.a, color: pal.graph2 },
            ]);
            graph_footer(ui, pal, cx.interval_ms);
            ui.add_space(space::M);
            let avg = |r: &Ring<f32>| if r.is_empty() { 0.0 } else { r.iter().map(|v| *v as f64).sum::<f64>() / r.len() as f64 };
            let items = [
                ("受信", format::speed(Some(n.recv_bps))),
                ("送信", format::speed(Some(n.send_bps))),
                ("受信の最大", format::speed(Some(ring_max(&s.b) as f64))),
                ("送信の最大", format::speed(Some(ring_max(&s.a) as f64))),
                ("受信の平均", format::speed(Some(avg(&s.b)))),
                ("送信の平均", format::speed(Some(avg(&s.a)))),
            ];
            // 受信が大きいと送信の線が底に張り付くので、送信だけの目盛りのグラフも並べる
            let send_max = nice_speed_max(ring_max(&s.a) as f64);
            two_up(
                ui,
                |ui| {
                    card_full(ui, pal, |ui| {
                        widgets::section(ui, pal, "使用状況");
                        stat_grid(ui, pal, &items);
                    });
                },
                |ui| {
                    card_full(ui, pal, |ui| {
                        widgets::section(ui, pal, "送信の速さ(送信だけの目盛り)");
                        graph(ui, pal, 120.0, send_max, Unit::Speed, &[Line { name: "送信", ring: &s.a, color: pal.graph2 }]);
                    });
                },
            );
            None
        }
        Page::Gpu(luid) => {
            header(ui, pal, "GPU", &gpu_name(&p, &luid));
            let g = p.gpus.iter().flatten().find(|g| g.luid == luid).cloned();
            let s = pv.gpus.get(&luid).unwrap_or(&empty);
            let Some(g) = g else {
                widgets::empty_state(ui, pal, UNAVAILABLE, None);
                return None;
            };
            hero(ui, pal, &format!("{:.0}%", g.util_pct), "使用率");
            graph_caption(ui, pal, "使用率", &[]);
            graph(ui, pal, cx.graph_h, 100.0, Unit::Pct, &[Line { name: "使用率", ring: &s.a, color: pal.graph }]);
            graph_footer(ui, pal, cx.interval_ms);
            ui.add_space(space::M);
            let items = [
                ("使用率", format!("{:.0}%", g.util_pct)),
                ("専用メモリ", format::opt_bytes(g.dedicated_bytes)),
                ("共有メモリ", format::opt_bytes(g.shared_bytes)),
                ("使用率の最大", format!("{:.0}%", ring_max(&s.a))),
            ];
            let spec = TopSpec { title: "GPU を使っているプロセス(全 GPU)", col: "GPU", rows: &pv.tops.gpu, unit: Unit::Pct, full: Some(100.0) };
            two_up(
                ui,
                |ui| {
                    card_full(ui, pal, |ui| {
                        widgets::section(ui, pal, "使用状況");
                        stat_grid(ui, pal, &items);
                    });
                },
                |ui| card_full(ui, pal, |ui| top_table(ui, pal, &spec)),
            )
        }
        Page::NoDisk => {
            header(ui, pal, "ディスク", "");
            widgets::empty_state(ui, pal, UNAVAILABLE, None);
            None
        }
        Page::NoNet => {
            header(ui, pal, "ネットワーク", "");
            widgets::empty_state(ui, pal, UNAVAILABLE, None);
            None
        }
        Page::NoGpu => {
            header(ui, pal, "GPU", "");
            widgets::empty_state(ui, pal, "この PC では GPU の値を取得できません", None);
            None
        }
    }
}

fn cpu_page(ui: &mut Ui, cx: &Ctx<'_>, pv: &mut PerfView, p: &PerfSample) -> Option<ProcId> {
    let pal = &cx.pal;
    header(ui, pal, "CPU", cx.env.cpu_name.as_deref().unwrap_or(""));
    if p.cpu_total.is_none() && pv.cpu.is_empty() {
        widgets::empty_state(ui, pal, UNAVAILABLE, None);
        return None;
    }
    hero(ui, pal, &format::pct(p.cpu_total), "使用率");
    ui.horizontal(|ui| {
        ui.label(RichText::new(if pv.per_core { "論理プロセッサごとの使用率" } else { "全体の使用率" }).color(pal.weak).size(size::SMALL));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.selectable_value(&mut pv.per_core, true, "論理プロセッサごと");
            ui.selectable_value(&mut pv.per_core, false, "全体");
        });
    });
    if pv.per_core {
        core_grid(ui, pal, pv, cx.graph_h);
    } else {
        graph(ui, pal, cx.graph_h, 100.0, Unit::Pct, &[Line { name: "CPU", ring: &pv.cpu, color: pal.graph }]);
    }
    graph_footer(ui, pal, cx.interval_ms);
    ui.add_space(space::M);
    let usage = [
        ("使用率", format::pct(p.cpu_total)),
        ("稼働時間", format::uptime(p.uptime_ms)),
        ("プロセス", format::thousands(cx.totals.processes as u64)),
        ("スレッド", format::thousands(cx.totals.threads as u64)),
        ("ハンドル", format::thousands(cx.totals.handles)),
    ];
    let mut build = Vec::new();
    if let Some(c) = &cx.env.cpu_info {
        build.push(("ソケット", c.sockets.to_string()));
        build.push(("コア", c.cores.to_string()));
        build.push(("論理プロセッサ", c.logical.to_string()));
        for (k, v) in [("L1 キャッシュ", c.l1), ("L2 キャッシュ", c.l2), ("L3 キャッシュ", c.l3)] {
            if v > 0 {
                build.push((k, format::bytes(v)));
            }
        }
    }
    let spec = TopSpec { title: "CPU を使っているプロセス", col: "CPU", rows: &pv.tops.cpu, unit: Unit::Pct, full: Some(100.0) };
    two_up(
        ui,
        |ui| {
            card_full(ui, pal, |ui| {
                widgets::section(ui, pal, "使用状況");
                stat_grid(ui, pal, &usage);
                if !build.is_empty() {
                    widgets::section(ui, pal, "構成");
                    stat_grid(ui, pal, &build);
                }
            });
        },
        |ui| card_full(ui, pal, |ui| top_table(ui, pal, &spec)),
    )
}

/// FR-24: 論理プロセッサごとの格子。各セルに番号と今の %、小さなグラフ。数が多ければ縦に伸びる(外のスクロールで見る)
fn core_grid(ui: &mut Ui, pal: &Palette, pv: &PerfView, graph_h: f32) {
    let n = pv.cores.len();
    if n == 0 {
        widgets::empty_state(ui, pal, UNAVAILABLE, None);
        return;
    }
    let gap = space::S + 2.0;
    let w = ui.available_width();
    // 横:縦が 1.8 くらいになる行の数を選ぶ(幅は 110 以上、高さは 52 以上。足りなければ縦に伸びる)
    let shape = |rows: usize| {
        let cols = n.div_ceil(rows);
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = ((graph_h - gap * (rows as f32 - 1.0)) / rows as f32).max(52.0);
        (cols, cw, ch)
    };
    let rows = (1..=n)
        .filter(|r| shape(*r).1 >= 110.0 || *r == n)
        .min_by(|a, b| {
            let score = |r: usize| {
                let (_, cw, ch) = shape(r);
                (cw / ch / 1.8).ln().abs()
            };
            score(*a).total_cmp(&score(*b))
        })
        .unwrap_or(1);
    let (cols, cell_w, cell_h) = shape(rows);
    let total_h = cell_h * rows as f32 + gap * (rows as f32 - 1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, total_h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let small = FontId::proportional(size::SMALL);
    let text = ui.visuals().text_color();
    for (i, r) in pv.cores.iter().enumerate() {
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let cell = Rect::from_min_size(egui::pos2(rect.left() + cx * (cell_w + gap), rect.top() + cy * (cell_h + gap)), egui::vec2(cell_w, cell_h));
        if !ui.is_rect_visible(cell) {
            continue;
        }
        let now = r.last().copied();
        let painter = ui.painter();
        painter.rect_filled(cell, CornerRadius::same(4), pal.surface);
        painter.rect_stroke(cell, CornerRadius::same(4), Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        let head_h = 18.0;
        painter.text(egui::pos2(cell.left() + space::S + 2.0, cell.top() + head_h / 2.0 + 1.0), Align2::LEFT_CENTER, format!("CPU {i}"), small.clone(), pal.weak);
        painter.text(egui::pos2(cell.right() - space::S - 2.0, cell.top() + head_h / 2.0 + 1.0), Align2::RIGHT_CENTER, now.map_or_else(|| format::DASH.to_string(), |v| format!("{v:.0}%")), small.clone(), text);
        let plot = Rect::from_min_max(egui::pos2(cell.left() + 1.0, cell.top() + head_h + 2.0), egui::pos2(cell.right() - 1.0, cell.bottom() - 2.0));
        let clip = ui.painter_at(plot);
        clip.hline(plot.x_range(), plot.center().y, Stroke::new(1.0, widgets::tint(pal.border, 150)));
        paint_line(&clip, plot, r, 100.0, pal.graph, 48, 1.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_speed_max_rounds_up_within_unit() {
        assert_eq!(nice_speed_max(0.0), 1024.0);
        assert_eq!(nice_speed_max(3.0 * 1024.0), 4.0 * 1024.0);
        assert_eq!(nice_speed_max(4.0 * 1024.0), 6.0 * 1024.0, "上の縁に張り付かない");
        assert_eq!(nice_speed_max(1.5 * 1024.0 * 1024.0), 2.0 * 1024.0 * 1024.0);
        assert_eq!(nice_speed_max(70.0 * 1024.0), 80.0 * 1024.0);
        assert_eq!(nice_speed_max(850.0 * 1024.0), 1000.0 * 1024.0);
        assert_eq!(nice_speed_max(930.0 * 1024.0), 1024.0 * 1024.0, "1.95 MB/s にならない");
    }

    #[test]
    fn axis_labels_share_the_unit_of_max() {
        let gb = 1024f64.powi(3);
        assert_eq!(Unit::Bytes.axis(0.0, 32.0 * gb), "0");
        assert_eq!(Unit::Bytes.axis(8.0 * gb, 32.0 * gb), "8 GB");
        assert_eq!(Unit::Speed.axis(0.5 * 1024.0 * 1024.0, 2.0 * 1024.0 * 1024.0), "0.5 MB/s");
        assert_eq!(Unit::Pct.axis(75.0, 100.0), "75%");
        assert_eq!(Unit::Speed.axis(0.25 * 1024.0, 1024.0), "0.25 KB/s");
        assert_eq!(Unit::Speed.axis(4.5 * 1024.0, 6.0 * 1024.0), "4.5 KB/s");
    }

    #[test]
    fn tops_skip_idle_gone_and_zero() {
        let s = crate::demo::snapshot(5);
        let t = Tops::from_rows(&s.rows);
        assert!(t.cpu.len() <= TOP_N && !t.cpu.is_empty());
        assert!(t.cpu.iter().all(|r| r.id.pid != 0 && r.value > 0.0));
        assert!(t.cpu.windows(2).all(|w| w[0].value >= w[1].value));
        assert!(t.mem.iter().all(|r| r.name != "updater.exe"), "終わった行は出さない");
        assert_eq!(t.mem[0].name, "AceGame.exe");
    }
}
