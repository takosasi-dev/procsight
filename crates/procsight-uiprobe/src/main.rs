//! Phase 0 の画面の確かめ(仕様書 §9.6)。日本語のフォントの書体の一覧と見本・IME の欄・1,000 行 × 9 列の表・トレイへ隠して戻す。
//! 利用者が自分の PC で動かす物(自動のテストでは起動しない)。窓の下に描画の種類・アダプターの名前・1 フレームの時間の中央値を出す。
//! フォントの名前の読み手とトレイの絵は procsight-app のファイルを #[path] で読む(依存を足さないため)。

#[path = "../../procsight-app/src/ttc.rs"]
mod ttc;

#[allow(dead_code)]
#[path = "../../procsight-app/src/icon_art.rs"]
mod icon_art;

use eframe::egui;
use egui::{FontData, FontDefinitions, FontFamily, RichText};
use egui_extras::{Column, TableBuilder};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// 名前がこれで始まるファイルを見る(§9.6)
const PREFIXES: [&str; 4] = ["yu", "meiryo", "msgothic", "bizud"];

struct FontFile {
    path: String,
    faces: Vec<(u32, String)>,
}

struct Probe {
    files: Vec<FontFile>,
    /// (ファイルの位置, 書体の番号)
    chosen: Option<(usize, u32)>,
    loaded: Option<(usize, u32)>,
    search: String,
    seed: u64,
    last_tick: Instant,
    frame_ms: Vec<f32>,
    tray: Option<(TrayIcon, MenuId)>,
    glow_requested: bool,
    renderer: String,
}

fn list_fonts() -> Vec<FontFile> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let dir = format!("{}\\Fonts", root.trim_end_matches('\\'));
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let ext_ok = name.ends_with(".ttc") || name.ends_with(".ttf");
            if !ext_ok || !PREFIXES.iter().any(|p| name.starts_with(p)) {
                continue;
            }
            let path = e.path().to_string_lossy().to_string();
            let faces = std::fs::read(&path).map(|b| ttc::faces(&b)).unwrap_or_default();
            out.push(FontFile { path, faces });
        }
    }
    out.sort_by_key(|f| f.path.to_lowercase());
    out
}

impl Probe {
    fn new(cc: &eframe::CreationContext<'_>) -> Probe {
        let files = list_fonts();
        // 最初は Yu Gothic UI(無ければ最初の書体)
        let chosen = files
            .iter()
            .enumerate()
            .find_map(|(i, f)| f.faces.iter().find(|(_, n)| n == "Yu Gothic UI Regular").map(|(k, _)| (i, *k)))
            .or_else(|| files.first().and_then(|f| f.faces.first().map(|(k, _)| (0, *k))));
        let renderer = match cc.wgpu_render_state.as_ref() {
            Some(rs) => {
                let info = rs.adapter.get_info();
                format!("wgpu({:?})  アダプター: {}", info.backend, info.name)
            }
            None => "wgpu 以外".into(),
        };
        let mut p = Probe {
            files,
            chosen,
            loaded: None,
            search: String::new(),
            seed: 1,
            last_tick: Instant::now(),
            frame_ms: Vec::new(),
            tray: None,
            glow_requested: std::env::args().any(|a| a == "--glow"),
            renderer,
        };
        p.apply_font(&cc.egui_ctx);
        p
    }

    fn apply_font(&mut self, ctx: &egui::Context) {
        let Some((fi, idx)) = self.chosen else { return };
        if self.loaded == Some((fi, idx)) {
            return;
        }
        let Ok(bytes) = std::fs::read(&self.files[fi].path) else { return };
        let mut defs = FontDefinitions::empty();
        defs.font_data.insert("probe".into(), Arc::new(FontData::from_blob(Arc::new(bytes), idx)));
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families.insert(fam, vec!["probe".into()]);
        }
        ctx.set_fonts(defs);
        self.loaded = Some((fi, idx));
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            let open = MenuItem::new("開く", true, None);
            let menu = Menu::new();
            if menu.append(&open).is_err() {
                return;
            }
            let Ok(icon) = Icon::from_rgba(icon_art::tray_cpu_rgba(40.0), 16, 16) else { return };
            match TrayIconBuilder::new().with_tooltip("ProcSight UI Probe").with_icon(icon).with_menu(Box::new(menu)).build() {
                Ok(t) => self.tray = Some((t, open.id().clone())),
                Err(_) => return,
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }
}

fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[s.len() / 2]
}

/// 1,000 行 × 9 列の値(seed で毎秒変える)
fn cell(seed: u64, row: usize, col: usize) -> String {
    let x = (row as u64 * 2_654_435_761 + col as u64 * 40_503 + seed * 97) % 100_000;
    match col {
        0 => format!("process-{row:04}.exe"),
        1 => (1000 + row * 4).to_string(),
        2 => ["実行中", "一時停止中", "応答なし", "触れない"][x as usize % 4].into(),
        3 => format!("{:.1}%", x as f64 / 1000.0),
        4 => format!("{:.1} MB", x as f64 / 100.0),
        5 | 6 => format!("{:.1} KB/s", x as f64 / 300.0),
        7 => format!("{:.1}%", (x % 1000) as f64 / 100.0),
        _ => "user".into(),
    }
}

impl eframe::App for Probe {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some((_, open)) = &self.tray {
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                if e.id == *open {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        if self.last_tick.elapsed() >= Duration::from_secs(1) {
            self.seed += 1;
            self.last_tick = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        self.apply_font(ui.ctx());
        egui::Panel::bottom("diag").show(ui, |ui| {
            ui.label(format!("描画: {}   1 フレームの時間の中央値: {:.2} ms", self.renderer, median(&self.frame_ms)));
            if self.glow_requested {
                ui.label("--glow: このビルドには glow の機能が入っていないため、wgpu で描いています");
            }
        });
        egui::Panel::left("fonts").resizable(true).default_size(360.0).show(ui, |ui| {
            ui.heading("フォント");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (fi, f) in self.files.iter().enumerate() {
                    let name = f.path.rsplit('\\').next().unwrap_or(&f.path);
                    ui.label(RichText::new(format!("{name}(書体 {} 個)", f.faces.len())).strong());
                    for (idx, face) in &f.faces {
                        let sel = self.chosen == Some((fi, *idx));
                        if ui.selectable_label(sel, format!("  {idx}: {face}")).clicked() {
                            self.chosen = Some((fi, *idx));
                        }
                    }
                }
                if self.files.is_empty() {
                    ui.label("候補のフォントが見つかりません");
                }
            });
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.label(RichText::new("プロセスを終了しますか。応答なし・一時停止中・触れない").size(20.0));
            ui.label(RichText::new("1,234.5 MB/s").size(20.0));
            ui.horizontal(|ui| {
                let l = ui.label("検索:");
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("ここに IME で「ぷろせす」と入れる").desired_width(300.0))
                    .labelled_by(l.id);
                if ui.button("トレイへ隠す").clicked() {
                    self.hide_to_tray(ui.ctx());
                }
            });
            ui.separator();
            let seed = self.seed;
            let mut tb = TableBuilder::new(ui).striped(true).resizable(true).auto_shrink([false, false]);
            for w in [200.0, 70.0, 90.0, 70.0, 90.0, 90.0, 90.0, 60.0] {
                tb = tb.column(Column::initial(w).clip(true));
            }
            tb.column(Column::remainder())
                .header(20.0, |mut h| {
                    for t in ["名前", "PID", "状態", "CPU", "メモリ", "I/O 読み取り", "I/O 書き込み", "GPU", "ユーザー"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|body| {
                    body.rows(18.0, 1000, |mut row| {
                        let r = row.index();
                        for c in 0..9 {
                            row.col(|ui| {
                                ui.label(cell(seed, r, c));
                            });
                        }
                    });
                });
        });
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        if self.frame_ms.len() >= 200 {
            self.frame_ms.remove(0);
        }
        self.frame_ms.push(ms);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("ProcSight UI Probe").with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("ProcSight UI Probe", options, Box::new(|cc| Ok(Box::new(Probe::new(cc)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_and_median() {
        assert_eq!(cell(1, 3, 0), "process-0003.exe");
        assert_eq!(cell(1, 3, 1), "1012");
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[]), 0.0);
    }
}
