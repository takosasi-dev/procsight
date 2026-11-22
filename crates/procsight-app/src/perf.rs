//! S2 パフォーマンス(画面設計書 §5): 左にページの並び(名前・今の値・小さなグラフ)、右に大きいグラフ(egui_plot)と数字。
//! 履歴は Snapshot ごとに直近 60 回分だけメモリに持つ(非スコープ: 保存)。更新間隔を変えたら空から始める。
//! 取れないページは並びに残し、右に「この PC では取得できません」を出す(E-11)。

use eframe::egui;
use crate::app::State;
use crate::theme::{self, Palette};
use egui::{Color32, RichText, Sense, Ui, Vec2};
use egui_plot::{Line, Plot, PlotPoints};
use procsight_core::format;
use procsight_core::ring::Ring;
use procsight_core::{PerfSample, Snapshot};
use std::collections::BTreeMap;

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
}

impl PerfView {
    pub fn clear(&mut self) {
        let page = std::mem::take(&mut self.page);
        let per_core = self.per_core;
        *self = PerfView { page, per_core, ..Default::default() };
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
    }
}

/// 小さな線のグラフ(枠つき)。`max` を上端にする
pub fn sparkline(ui: &mut Ui, values: impl Iterator<Item = f32>, max: f32, size: Vec2, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    let vals: Vec<f32> = values.collect();
    if vals.len() < 2 || max <= 0.0 {
        return;
    }
    let n = procsight_core::ring::GRAPH_POINTS as f32 - 1.0;
    let start = procsight_core::ring::GRAPH_POINTS - vals.len().min(procsight_core::ring::GRAPH_POINTS);
    let pts: Vec<egui::Pos2> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + (start + i) as f32 / n * rect.width();
            let y = rect.bottom() - (v / max).clamp(0.0, 1.0) * (rect.height() - 2.0) - 1.0;
            egui::pos2(x, y)
        })
        .collect();
    for w in pts.windows(2) {
        let quad = vec![w[0], w[1], egui::pos2(w[1].x, rect.bottom()), egui::pos2(w[0].x, rect.bottom())];
        painter.add(egui::Shape::convex_polygon(quad, color.gamma_multiply(0.18), egui::Stroke::NONE));
    }
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
}

fn ring_max(r: &Ring<f32>) -> f32 {
    r.iter().copied().fold(0.0, f32::max)
}

fn points(r: &Ring<f32>, scale: f64) -> PlotPoints<'static> {
    let start = procsight_core::ring::GRAPH_POINTS - r.len();
    PlotPoints::from_iter(r.iter().enumerate().map(|(i, v)| [(start + i) as f64, *v as f64 * scale]))
}

fn big_plot(ui: &mut Ui, id: &str, height: f32, y_max: Option<f64>, lines: Vec<(String, &Ring<f32>, f64, Color32)>) {
    let mut plot = Plot::new(id)
        .height(height)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_x(false)
        .show_axes([false, true])
        .include_x(0.0)
        .include_x((procsight_core::ring::GRAPH_POINTS - 1) as f64)
        .include_y(0.0)
        .set_margin_fraction(egui::vec2(0.0, 0.05))
        .y_axis_min_width(36.0);
    if lines.len() > 1 {
        // 2 本の線は、どちらが何かを凡例で出す
        plot = plot.legend(egui_plot::Legend::default().position(egui_plot::Corner::LeftTop));
    }
    if let Some(m) = y_max {
        plot = plot.include_y(m).auto_bounds(egui::Vec2b::new(false, false)).default_y_bounds(0.0, m);
    }
    plot.show(ui, |p| {
        for (name, r, scale, color) in lines {
            p.line(Line::new(name, points(r, scale)).color(color).width(1.5).fill(0.0).fill_alpha(0.15));
        }
    });
}

fn side_item(ui: &mut Ui, sel: bool, name: &str, value: &str, vals: Option<(&Ring<f32>, f32)>, pal: &Palette) -> bool {
    let fill = if sel { ui.visuals().selection.bg_fill } else { Color32::TRANSPARENT };
    let resp = egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(RichText::new(value).color(pal.weak));
                });
                if let Some((r, max)) = vals {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        sparkline(ui, r.iter().copied(), max, egui::vec2(70.0, 30.0), pal.graph);
                    });
                }
            });
        })
        .response
        .interact(Sense::click());
    resp.clicked()
}

fn disk_label(name: &str) -> String {
    format!("ディスク {name}")
}

fn gpu_name(p: &PerfSample, luid: &str) -> String {
    p.gpus.iter().flatten().find(|g| g.luid == luid).map_or_else(|| luid.to_string(), |g| g.name.clone())
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    let pal = theme::palette(ui);
    let pv = &mut st.perf;
    let p = pv.last.clone();
    egui::Panel::left("perf_side").resizable(false).exact_size(240.0).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut pick = None;
            if side_item(ui, pv.page == Page::Cpu, "CPU", &format::pct(p.cpu_total), Some((&pv.cpu, 100.0)), &pal) {
                pick = Some(Page::Cpu);
            }
            let mem_v = p.mem.as_ref().and_then(|m| m.used_pct()).map_or_else(|| format::DASH.to_string(), |v| format!("{v:.0}%"));
            let mem_max = p.mem.as_ref().map_or(1.0, |m| m.total as f32);
            if side_item(ui, pv.page == Page::Memory, "メモリ", &mem_v, Some((&pv.mem, mem_max)), &pal) {
                pick = Some(Page::Memory);
            }
            match &p.disks {
                Some(ds) if !ds.is_empty() => {
                    for d in ds {
                        let page = Page::Disk(d.name.clone());
                        let ring = pv.disks.get(&d.name).map(|s| (&s.a, 100.0));
                        if side_item(ui, pv.page == page, &disk_label(&d.name), &format!("{:.0}%", d.active_pct), ring, &pal) {
                            pick = Some(page);
                        }
                    }
                }
                _ => {
                    if side_item(ui, pv.page == Page::NoDisk, "ディスク", format::DASH, None, &pal) {
                        pick = Some(Page::NoDisk);
                    }
                }
            }
            match &p.nets {
                Some(ns) if !ns.is_empty() => {
                    for n in ns {
                        let page = Page::Net(n.name.clone());
                        let ring = pv.nets.get(&n.name).map(|s| (&s.b, ring_max(&s.b).max(ring_max(&s.a)).max(1.0)));
                        let v = format!("↓{}", format::speed(Some(n.recv_bps)));
                        if side_item(ui, pv.page == page, &n.name, &v, ring, &pal) {
                            pick = Some(page);
                        }
                    }
                }
                _ => {
                    if side_item(ui, pv.page == Page::NoNet, "ネットワーク", format::DASH, None, &pal) {
                        pick = Some(Page::NoNet);
                    }
                }
            }
            match &p.gpus {
                Some(gs) if !gs.is_empty() => {
                    for g in gs {
                        let page = Page::Gpu(g.luid.clone());
                        let ring = pv.gpus.get(&g.luid).map(|s| (&s.a, 100.0));
                        if side_item(ui, pv.page == page, &g.name, &format!("{:.0}%", g.util_pct), ring, &pal) {
                            pick = Some(page);
                        }
                    }
                }
                _ => {
                    if side_item(ui, pv.page == Page::NoGpu, "GPU", format::DASH, None, &pal) {
                        pick = Some(Page::NoGpu);
                    }
                }
            }
            if let Some(pg) = pick {
                pv.page = pg;
            }
        });
    });
    let totals = st.snap.totals.clone();
    let cpu_name = st.env.cpu_name.clone().unwrap_or_default();
    let pv = &mut st.perf;
    egui::CentralPanel::default_margins().show(ui, |ui| {
        let h = (ui.available_height() * 0.5).max(160.0);
        match pv.page.clone() {
            Page::Cpu => {
                header(ui, "CPU", &cpu_name);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut pv.per_core, false, "全体");
                    ui.selectable_value(&mut pv.per_core, true, "論理プロセッサごと");
                });
                if p.cpu_total.is_none() && pv.cpu.is_empty() {
                    unavailable(ui, "この PC では取得できません");
                } else if pv.per_core {
                    core_grid(ui, pv, &pal);
                } else {
                    big_plot(ui, "cpu", h, Some(100.0), vec![("CPU".into(), &pv.cpu, 1.0, pal.graph)]);
                }
                ui.add_space(8.0);
                stats(ui, &pal, &[
                    ("使用率", format::pct(p.cpu_total)),
                    ("稼働時間", format::uptime(p.uptime_ms)),
                    ("プロセス", format::thousands(totals.processes as u64)),
                    ("スレッド", format::thousands(totals.threads as u64)),
                    ("ハンドル", format::thousands(totals.handles)),
                ]);
            }
            Page::Memory => {
                header(ui, "メモリ", &p.mem.as_ref().map_or_else(String::new, |m| format::bytes(m.total)));
                match &p.mem {
                    None => unavailable(ui, "この PC では取得できません"),
                    Some(m) => {
                        let gb = 1.0 / (1024.0 * 1024.0 * 1024.0);
                        big_plot(ui, "mem", h, Some(m.total as f64 * gb), vec![("使用中".into(), &pv.mem, gb, pal.graph)]);
                        ui.add_space(8.0);
                        stats(ui, &pal, &[
                            ("使用中", format::bytes(m.in_use())),
                            ("利用可能", format::bytes(m.available)),
                            ("コミット済み / 上限", format!("{} / {}", format::bytes(m.commit_total), format::bytes(m.commit_limit))),
                        ]);
                    }
                }
            }
            Page::Disk(name) => {
                header(ui, &disk_label(&name), "");
                let d = p.disks.iter().flatten().find(|d| d.name == name).cloned();
                match (d, pv.disks.get(&name)) {
                    (Some(d), Some(s)) => {
                        ui.label(RichText::new("アクティブな時間").color(pal.weak));
                        big_plot(ui, "disk_active", h * 0.6, Some(100.0), vec![("アクティブな時間".into(), &s.a, 1.0, pal.graph)]);
                        ui.label(RichText::new("読み取り・書き込みの速さ(MB/s)").color(pal.weak));
                        let mb = 1.0 / (1024.0 * 1024.0);
                        big_plot(ui, "disk_rw", h * 0.6, None, vec![
                            ("読み取り".into(), &s.b, mb, pal.graph),
                            ("書き込み".into(), &s.c, mb, pal.graph2),
                        ]);
                        stats(ui, &pal, &[
                            ("アクティブな時間", format!("{:.0}%", d.active_pct)),
                            ("読み取りの速さ", format::speed(Some(d.read_bps))),
                            ("書き込みの速さ", format::speed(Some(d.write_bps))),
                        ]);
                    }
                    _ => unavailable(ui, "この PC では取得できません"),
                }
            }
            Page::Net(name) => {
                header(ui, &name, "");
                let n = p.nets.iter().flatten().find(|n| n.name == name).cloned();
                match (n, pv.nets.get(&name)) {
                    (Some(n), Some(s)) => {
                        ui.label(RichText::new("送信と受信の速さ(KB/s)").color(pal.weak));
                        let kb = 1.0 / 1024.0;
                        big_plot(ui, "net", h, None, vec![("送信".into(), &s.a, kb, pal.graph2), ("受信".into(), &s.b, kb, pal.graph)]);
                        stats(ui, &pal, &[("送信", format::speed(Some(n.send_bps))), ("受信", format::speed(Some(n.recv_bps)))]);
                    }
                    _ => unavailable(ui, "この PC では取得できません"),
                }
            }
            Page::Gpu(luid) => {
                header(ui, &gpu_name(&p, &luid), "");
                let g = p.gpus.iter().flatten().find(|g| g.luid == luid).cloned();
                match (g, pv.gpus.get(&luid)) {
                    (Some(g), Some(s)) => {
                        big_plot(ui, "gpu", h, Some(100.0), vec![("使用率".into(), &s.a, 1.0, pal.graph)]);
                        stats(ui, &pal, &[
                            ("使用率", format!("{:.0}%", g.util_pct)),
                            ("専用メモリ", format::opt_bytes(g.dedicated_bytes)),
                            ("共有メモリ", format::opt_bytes(g.shared_bytes)),
                        ]);
                    }
                    _ => unavailable(ui, "この PC では取得できません"),
                }
            }
            Page::NoDisk => {
                header(ui, "ディスク", "");
                unavailable(ui, "この PC では取得できません");
            }
            Page::NoNet => {
                header(ui, "ネットワーク", "");
                unavailable(ui, "この PC では取得できません");
            }
            Page::NoGpu => {
                header(ui, "GPU", "");
                unavailable(ui, "この PC では GPU の値を取得できません");
            }
        }
    });
}

fn header(ui: &mut Ui, name: &str, right: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(20.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(right));
    });
}

fn unavailable(ui: &mut Ui, text: &str) {
    let pal = theme::palette(ui);
    ui.add_space(24.0);
    ui.label(RichText::new(text).color(pal.weak));
}

fn stats(ui: &mut Ui, pal: &Palette, items: &[(&str, String)]) {
    ui.horizontal_wrapped(|ui| {
        for (k, v) in items {
            ui.vertical(|ui| {
                ui.label(RichText::new(*k).color(pal.weak));
                ui.label(RichText::new(v).size(17.0));
            });
            ui.add_space(20.0);
        }
    });
}

/// FR-24: 論理プロセッサごとの格子。64 個までは格子、それを超えたらスクロール
fn core_grid(ui: &mut Ui, pv: &PerfView, pal: &Palette) {
    let n = pv.cores.len();
    if n == 0 {
        unavailable(ui, "この PC では取得できません");
        return;
    }
    let cols = ((n as f32).sqrt().ceil() as usize).clamp(1, 8);
    let gap = 6.0;
    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).max(60.0);
    let h = 56.0;
    egui::ScrollArea::vertical().max_height(if n > 64 { 360.0 } else { f32::INFINITY }).show(ui, |ui| {
        egui::Grid::new("cores").spacing([gap, gap]).show(ui, |ui| {
            for (i, r) in pv.cores.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("CPU {i}  {}", format::pct(r.last().copied()))).small().color(pal.weak));
                    sparkline(ui, r.iter().copied(), 100.0, egui::vec2(w, h), pal.graph);
                });
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
    });
}
