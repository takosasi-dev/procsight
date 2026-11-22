//! S7 このソフトについて(画面設計書 §7.5・FR-50)。製品名・版・MIT License・著作者・依存のライセンスの一覧。
//! 依存の一覧は licenses.txt を exe に埋め込む(Phase 8 で統合担当が中身を作る)。

use eframe::egui;
use crate::app::State;
use crate::theme;
use egui::{RichText, Ui};

pub const LICENSES: &str = include_str!("../licenses.txt");

pub fn window(st: &mut State, ui: &mut Ui) {
    if !st.about_open {
        return;
    }
    let pal = theme::palette(ui);
    let mut open = true;
    let ctx = ui.ctx().clone();
    egui::Window::new("このソフトについて").open(&mut open).collapsible(false).default_size([520.0, 420.0]).show(&ctx, |ui| {
        ui.label(RichText::new("ProcSight").size(22.0).strong());
        ui.label(format!("版 {}", env!("CARGO_PKG_VERSION")));
        ui.label("MIT License");
        ui.label(format!("著作者: {}", env!("CARGO_PKG_AUTHORS")));
        ui.add_space(6.0);
        ui.label(RichText::new("ProcSight はネットにつながりません。履歴を保存しません").color(pal.accent));
        ui.add_space(8.0);
        ui.label(RichText::new("依存のライセンス").strong());
        egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new(LICENSES).monospace());
        });
    });
    if !open {
        st.about_open = false;
    }
}
