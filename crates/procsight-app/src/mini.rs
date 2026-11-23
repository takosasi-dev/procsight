//! ミニ表示(v0.3.0): CPU・メモリ・GPU・ディスク・ネットの今の値と小さなグラフ、CPU の上位 1 つだけの小さな窓(v0.4.0: 出す行は cfg.mini_items)。
//! 窓の大きさ・常に手前の切り替えは window_commands(純粋な関数)で決め、shell.rs が egui に渡すだけ。
//! 確認の窓・ボタン付きの帯・パレットが出たら普通の表示に戻す(should_leave)。ミニ表示は保存しない。

use crate::app::State;
use crate::perf::Series;
use crate::theme::{self, Palette, size, space};
use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, ViewportCommand, WindowLevel};
use procsight_core::config::{MiniItems, SpeedUnit};
use procsight_core::format;
use procsight_core::ring::Ring;
use procsight_core::{PerfSample, ProcState};

/// ミニ表示に入ったときの大きさ(点。文字の大きさを変えても中の並びは同じ)
pub const MINI_SIZE: Vec2 = Vec2::new(300.0, 230.0);
/// ミニ表示の間の最小の大きさ
pub const MINI_MIN: Vec2 = Vec2::new(240.0, 190.0);
/// 普通の表示の最小の大きさ(shell::run の with_min_inner_size と同じ。こちらは文字の大きさを掛ける前の値)
pub const NORMAL_MIN: Vec2 = Vec2::new(640.0, 400.0);

/// 窓に起きる変化
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowChange {
    /// 起動の直後に 1 回
    Startup { always_on_top: bool },
    AlwaysOnTop(bool),
    EnterMini,
    ExitMini,
}

/// 窓の命令を決めるのに要る事実。位置と大きさは egui の点(文字の大きさを掛けた後)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowFacts {
    /// 普通の表示の外側の左上(最大化の前の値)。使えない値なら None
    pub pos: Option<Pos2>,
    /// 普通の表示の中の大きさ(最大化の前の値)
    pub size: Vec2,
    pub maximized: bool,
    /// 文字の大きさ(egui の zoom)。最小の大きさを起動時と同じ実寸に戻すのに使う
    pub zoom: f32,
    /// このセッションで前にミニ表示を置いた場所(左上と大きさ)。保存しない
    pub mini: Option<(Pos2, Vec2)>,
}

/// 変化 → 窓に渡す命令の並び。入るときは最大化を外し、最小を下げてから縮める。出るときは大きさを戻してから最小を上げ直し、最後に最大化に戻す
pub fn window_commands(change: WindowChange, f: &WindowFacts) -> Vec<ViewportCommand> {
    let level = |on: bool| ViewportCommand::WindowLevel(if on { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
    match change {
        WindowChange::Startup { always_on_top } => if always_on_top { vec![level(true)] } else { Vec::new() },
        WindowChange::AlwaysOnTop(on) => vec![level(on)],
        WindowChange::EnterMini => {
            let mut v = Vec::new();
            if f.maximized {
                v.push(ViewportCommand::Maximized(false));
            }
            v.push(ViewportCommand::MinInnerSize(MINI_MIN));
            let (pos, size) = f.mini.map_or((None, MINI_SIZE), |(p, s)| (Some(p), s.max(MINI_MIN)));
            v.push(ViewportCommand::InnerSize(size));
            if let Some(p) = pos {
                v.push(ViewportCommand::OuterPosition(p));
            }
            v
        }
        WindowChange::ExitMini => {
            let zoom = if f.zoom.is_finite() && f.zoom > 0.0 { f.zoom } else { 1.0 };
            let mut v = vec![ViewportCommand::InnerSize(f.size), ViewportCommand::MinInnerSize(NORMAL_MIN / zoom)];
            if let Some(p) = f.pos {
                v.push(ViewportCommand::OuterPosition(p));
            }
            if f.maximized {
                v.push(ViewportCommand::Maximized(true));
            }
            v
        }
    }
}

/// ミニから出た後、窓が戻りきったか(戻りきるまで shell は cfg.window に書かない。縮んだままの値を覚えないため)
pub fn geometry_settled(want: &WindowFacts, inner: Vec2, maximized: bool) -> bool {
    if want.maximized {
        maximized
    } else {
        !maximized && (inner - want.size).abs().max_elem() < 2.0
    }
}

/// 普通の表示に戻すべきか: 確認の窓・ボタン付きの帯(小さな窓では押せない)・パレット・ショートカットの一覧
pub fn should_leave(st: &State) -> bool {
    st.dialog.is_some() || st.bands.iter().any(|b| !b.buttons.is_empty()) || st.palette.open || st.palette.help_open
}

/// 動いている触れないプロセスの数(状態の行と同じ数え方)
fn touch_free_running(st: &State) -> usize {
    st.snap.rows.iter().filter(|r| r.state == ProcState::TouchFree && r.is_gone == 0).count()
}

/// いくつかのリングを右(今)にそろえて、長さ points の 1 本にまとめる(f で合わせる)。返すのは (値, 埋まった数)。埋まっていない左側は NaN
fn combine<'a>(rings: impl Iterator<Item = &'a Ring<f32>>, points: usize, f: fn(f32, f32) -> f32) -> (Vec<f32>, usize) {
    let mut buf = vec![f32::NAN; points];
    let mut n = 0;
    for r in rings {
        for (j, v) in r.iter().rev().take(points).enumerate() {
            let slot = &mut buf[points - 1 - j];
            let v = if v.is_finite() { *v } else { 0.0 };
            *slot = if slot.is_nan() { v } else { f(*slot, v) };
            n = n.max(j + 1);
        }
    }
    (buf, n)
}

/// 1 行の中身
struct Row {
    name: &'static str,
    value: String,
    hover: String,
    /// 線(右が今。長さは点の数)と埋まった数と上端の値。None は取れない(枠だけ描く)
    line: Option<(Vec<f32>, usize, f32)>,
    color: Color32,
    /// 値を警告の色にする(90% 以上)
    hot: bool,
}

fn pct_row(name: &'static str, now: Option<f32>, line: (Vec<f32>, usize), hover: String, color: Color32) -> Row {
    match now {
        Some(v) => Row { name, value: format!("{v:.0}%"), hover, line: Some((line.0, line.1, 100.0)), color, hot: v >= 90.0 },
        None => Row { name, value: format::DASH.into(), hover, line: None, color, hot: false },
    }
}

/// 出す行だけを作る(cfg.mini_items。並びは CPU・メモリ・GPU・ディスク・受信・送信のまま)
fn rows(st: &State, pal: &Palette) -> Vec<Row> {
    let pv = &st.perf;
    let p: &PerfSample = &pv.last;
    let show: &MiniItems = &st.cfg.mini_items;
    let points = pv.points();
    let one = |r: &Ring<f32>| combine(std::iter::once(r), points, f32::max);
    let mut out = Vec::with_capacity(6);
    if show.cpu {
        out.push(pct_row("CPU", p.cpu_total, one(&pv.cpu), "CPU 全体の使用率".into(), pal.graph));
    }
    if show.mem {
        out.push(match &p.mem {
            Some(m) => {
                let hover = format!("使用中 {} / {}", format::bytes(m.in_use()), format::bytes(m.total));
                let (buf, n) = one(&pv.mem);
                let pct = m.used_pct();
                let value = pct.map_or_else(|| format::DASH.to_string(), |v| format!("{v:.0}%"));
                Row { name: "メモリ", value, hover, line: Some((buf, n, (m.total as f32).max(1.0))), color: pal.graph, hot: pct.is_some_and(|v| v >= 90.0) }
            }
            None => Row { name: "メモリ", value: format::DASH.into(), hover: "取得できません".into(), line: None, color: pal.graph, hot: false },
        });
    }
    if show.gpu {
        let gpu_now = p.gpus.as_ref().and_then(|g| g.iter().map(|g| g.util_pct).reduce(f32::max));
        let gpu_hover = match p.gpus.as_deref() {
            Some([g]) => g.name.clone(),
            Some(gs) if !gs.is_empty() => format!("{} 個の GPU のうち一番高い値", gs.len()),
            _ => "取得できません".into(),
        };
        out.push(pct_row("GPU", gpu_now, combine(pv.gpus.values().map(|s| &s.a), points, f32::max), gpu_hover, pal.graph));
    }
    if show.disk {
        let busiest = p.disks.as_ref().and_then(|d| d.iter().max_by(|a, b| a.active_pct.total_cmp(&b.active_pct)));
        let disk_hover = busiest.map_or_else(|| "取得できません".into(), |d| format!("一番忙しいディスク: {}(アクティブな時間)", d.name));
        out.push(pct_row("ディスク", busiest.map(|d| d.active_pct), combine(pv.disks.values().map(|s| &s.a), points, f32::max), disk_hover, pal.graph));
    }
    // ネットは全部のアダプターの合計。上端は線ごとに見えている最大(送信は受信より小さいことが多く、同じ上端では平らに見える)
    let nets = p.nets.as_ref().filter(|n| !n.is_empty());
    let net_max = |l: &[f32]| (l.iter().copied().filter(|v| v.is_finite()).fold(0.0, f32::max) * 1.1).max(1024.0);
    let net_hover = nets.map_or_else(|| "取得できません".into(), |n| format!("ネットワーク {} 個の合計", n.len()));
    type Pick = fn(&Series) -> &Ring<f32>;
    for (on, name, now, pick, color) in [
        (show.net_recv, "受信", nets.map(|n| n.iter().map(|x| x.recv_bps).sum::<f64>()), (|s| &s.b) as Pick, pal.graph),
        (show.net_send, "送信", nets.map(|n| n.iter().map(|x| x.send_bps).sum::<f64>()), (|s| &s.a) as Pick, pal.graph2),
    ] {
        if !on {
            continue;
        }
        let (buf, n) = combine(pv.nets.values().map(pick), points, |a, b| a + b);
        let max = net_max(&buf);
        out.push(Row { name, value: format::speed(now), hover: net_hover.clone(), line: now.map(|_| (buf, n, max)), color, hot: false });
    }
    out
}

/// 1 行に収まらない文字は「…」で切る
fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    ui.painter().layout_job(job)
}

/// 列の幅(名前・値・線)。窓の幅と文字の大きさだけで決め、値が変わっても線の幅は動かさない
struct Cols {
    name_w: f32,
    value_w: f32,
    /// 線を描く幅(狭すぎれば 0 で描かない)
    spark_w: f32,
}

fn cols(ui: &Ui, width: f32) -> Cols {
    let body = FontId::proportional(size::BODY);
    let w = |s: &str| ui.painter().layout_no_wrap(s.to_string(), body.clone(), Color32::WHITE).size().x;
    let name_w = ["CPU", "メモリ", "GPU", "ディスク", "受信", "送信", "上位"].iter().map(|s| w(s)).fold(0.0, f32::max);
    // 値の一番長い形(ビットのときは「Mbps」が「KB/s」より広い)
    let value_w = w(if format::speed_unit() == SpeedUnit::Bits { "999.9 Mbps" } else { "999.9 KB/s" });
    let rest = width - name_w - value_w - space::M * 2.0;
    Cols { name_w, value_w, spark_w: if rest >= 40.0 { rest } else { 0.0 } }
}

fn metric_row(ui: &mut Ui, pal: &Palette, c: &Cols, row_h: f32, points: usize, r: &Row) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{} {}", r.name, r.value)));
    if ui.is_rect_visible(rect) {
        let body = FontId::proportional(size::BODY);
        let y = rect.center().y;
        let name = galley(ui, r.name, body.clone(), pal.weak, c.name_w + 1.0);
        ui.painter().galley(Pos2::new(rect.left(), y - name.size().y / 2.0), name, pal.weak);
        let fg = if r.hot { pal.warn } else { ui.visuals().strong_text_color() };
        let value = galley(ui, &r.value, body, fg, c.value_w);
        let vx = rect.left() + c.name_w + space::M + c.value_w - value.size().x;
        ui.painter().galley(Pos2::new(vx, y - value.size().y / 2.0), value, fg);
        if c.spark_w > 0.0 {
            let sh = (row_h - 6.0).max(10.0);
            let sr = Rect::from_min_size(Pos2::new(rect.right() - c.spark_w, y - sh / 2.0), Vec2::new(c.spark_w, sh));
            // 取れない行は枠だけ(値の無い線)
            let (vals, max): (&[f32], f32) = r.line.as_ref().map_or((&[], 1.0), |(buf, n, max)| (&buf[buf.len() - n..], *max));
            ui.scope_builder(UiBuilder::new().max_rect(sr), |ui| {
                crate::perf::sparkline_in(ui, vals.iter().copied(), points, max, sr.size(), r.color);
            });
        }
    }
    resp.on_hover_text(&r.hover);
}

/// 上位の行の % の文字。CPU の数え方(コアごとなら論理プロセッサの数)を掛ける。並びは掛ける前の値のまま
fn top_value_text(st: &State) -> String {
    let factor = f64::from(st.cfg.cpu_factor(st.snap.logical_cpus));
    st.perf.tops.cpu.first().map_or_else(|| format::DASH.to_string(), |t| format!("{:.0}%", t.value * factor))
}

/// CPU を一番使っているプロセス(PID 0 と終わった行は perf の上位で除いてある)。押すと普通の表示のその行へ
fn top_row(st: &mut State, ui: &mut Ui, pal: &Palette, c: &Cols, row_h: f32) {
    let top = st.perf.tops.cpu.first().cloned();
    let w = ui.available_width();
    let sense = if top.is_some() { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), sense);
    let label = top.as_ref().map_or_else(|| "上位 —".to_string(), |t| format!("上位 {}", t.name));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    if ui.is_rect_visible(rect) {
        if resp.hovered() && top.is_some() {
            ui.painter().rect_filled(rect.expand2(Vec2::new(space::XS, 0.0)), 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let body = FontId::proportional(size::BODY);
        let y = rect.center().y;
        let name = galley(ui, "上位", body.clone(), pal.weak, c.name_w + 1.0);
        ui.painter().galley(Pos2::new(rect.left(), y - name.size().y / 2.0), name, pal.weak);
        let text = ui.visuals().strong_text_color();
        let value = top_value_text(st);
        let vg = galley(ui, &value, body.clone(), text, c.value_w);
        let vx = rect.left() + c.name_w + space::M + c.value_w - vg.size().x;
        ui.painter().galley(Pos2::new(vx, y - vg.size().y / 2.0), vg, text);
        if let Some(t) = &top {
            let x = rect.left() + c.name_w + c.value_w + space::M * 2.0;
            let g = galley(ui, &t.name, body, ui.visuals().text_color(), rect.right() - x);
            ui.painter().galley(Pos2::new(x, y - g.size().y / 2.0), g, text);
        }
    }
    if let Some(t) = top {
        let resp = resp.on_hover_text("CPU を一番使っているプロセス。押すと一覧のその行へ移ります").on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            st.set_mini(false);
            st.procs.scroll_to = Some(t.id);
            st.tab = crate::app::Tab::Processes;
        }
    }
}

/// 下の列: 左に状態の札(入るだけ)、右に「手前」と「元に戻す」
fn footer(st: &mut State, ui: &mut Ui, pal: &Palette) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = space::S;
        if ui.button("元に戻す").on_hover_text("普通の表示に戻します(Ctrl+M・窓のダブルクリックでも戻ります)").clicked() {
            st.set_mini(false);
        }
        let on = st.cfg.always_on_top;
        if ui.add(egui::Button::new("手前").selected(on)).on_hover_text("常に手前に表示(設定の「全般」と同じ)").clicked() {
            st.set_always_on_top(!on);
        }
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::S;
            let touch = touch_free_running(st);
            let mut pills: Vec<(String, Color32, String)> = Vec::new();
            if st.paused {
                pills.push(("停止中".into(), pal.warn, "更新を一時停止しています".into()));
            }
            if touch > 0 {
                pills.push((format!("触れない {touch}"), pal.accent, format!("触れないプロセス {touch} 個が動作中")));
            }
            if !st.paused && st.slow.active && st.effective_interval_ms != st.cfg.interval_ms {
                let t = crate::app::interval_label(st.effective_interval_ms);
                pills.push((format!("{t}ごと"), pal.accent, format!("ゲーム中のため{t}ごとに更新")));
            }
            let small = FontId::proportional(size::SMALL);
            for (text, color, hover) in pills {
                // 札の幅(左右の余白 7 ずつ)が残りに入らなければ出さない(はみ出して重ねない)
                let need = ui.painter().layout_no_wrap(text.clone(), small.clone(), color).size().x + 14.0;
                if need > ui.available_width() {
                    break;
                }
                crate::widgets::pill(ui, text, color).on_hover_text(hover);
            }
        });
    });
}

/// ミニ表示の中身を描く(st.mini が true のとき、窓の中身の代わりに呼ばれる)
pub fn ui(st: &mut State, ui: &mut Ui) {
    if should_leave(st) {
        // 入ろうとした同じフレーム(ボタン付きの帯があるときの Ctrl+M など)は shell が命令を出さないので、描き直しを頼む
        st.set_mini(false);
        ui.ctx().request_repaint();
    }
    let pal = theme::palette(ui);
    // グラフの長さ(普通の表示の perf::ui と同じ輪を使う。ミニの間は perf::ui が走らないので、ここでも合わせる)
    let points = st.cfg.graph_points;
    st.perf.set_points(points);
    let rows = rows(st, &pal);
    let show_top = st.cfg.mini_items.top_process;
    let text_h = ui.text_style_height(&egui::TextStyle::Body);
    let foot_h = (text_h + ui.spacing().button_padding.y * 2.0).max(ui.spacing().interact_size.y);
    let body_h = (ui.available_height() - foot_h - space::S).max(0.0);
    // 区切りの線はグラフの行と上位の両方があるときだけ。その線と間の分
    let sep = show_top && !rows.is_empty();
    let sep_h = if sep { space::S + 1.0 + space::XS } else { 0.0 };
    let n_rows = rows.len() + usize::from(show_top);
    // 縦に余れば行を高く(線が見やすい)、足りなければ文字の高さまで詰めて、それでも入らなければ縦に送る。
    // 行が少なくても 36 より高くしない(行が間延びしない)。残りは下の列の上の空きになる
    let row_h = ((body_h - sep_h) / n_rows.max(1) as f32).clamp(text_h + 4.0, 36.0).floor();
    let c = cols(ui, ui.available_width() - space::S);
    let body = egui::ScrollArea::vertical().max_height(body_h).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for r in &rows {
            metric_row(ui, &pal, &c, row_h, points, r);
        }
        if sep {
            ui.add_space(space::S);
            let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, pal.border));
            ui.add_space(space::XS);
        }
        if show_top {
            top_row(st, ui, &pal, &c, row_h);
        }
    });
    // 値と線の上(下の列のボタンを除く)をダブルクリックで戻る。行は押せない部品なので、入力で見る
    let dbl = ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary) && i.pointer.interact_pos().is_some_and(|p| body.inner_rect.contains(p)));
    if dbl {
        st.set_mini(false);
    }
    ui.add_space(space::S);
    ui.allocate_ui(Vec2::new(ui.available_width(), foot_h), |ui| footer(st, ui, &pal));
}

#[cfg(test)]
mod tests {
    use super::*;
    use procsight_core::config::CpuScale;

    /// コアごとの数え方: 上位のプロセスの % は論理プロセッサの数を掛け、全体の CPU の行は掛けない。並びは変えない
    #[test]
    fn per_core_scales_only_the_top_process() {
        let mut st = crate::demo::state();
        let top = st.perf.tops.cpu[0].clone();
        assert_eq!(top_value_text(&st), format!("{:.0}%", top.value), "既定は掛けない");
        let pal = theme::palette_for(true, st.cfg.accent);
        let cpu_row = |st: &State| rows(st, &pal).into_iter().find(|r| r.name == "CPU").map(|r| r.value).unwrap();
        let total_before = cpu_row(&st);
        st.cfg.cpu_scale = CpuScale::PerCore;
        assert_eq!(st.snap.logical_cpus, 8);
        assert_eq!(top_value_text(&st), format!("{:.0}%", top.value * 8.0));
        assert_eq!(cpu_row(&st), total_before, "全体の CPU は掛けない");
        assert_eq!(st.perf.tops.cpu[0].id, top.id, "並びは変えない");
    }
}
