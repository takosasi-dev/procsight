//! S7 このソフトについて(画面設計書 §7.5・FR-50)。製品名・版・MIT License・著作者・依存のライセンスの一覧。
//! 依存の一覧は licenses.txt を exe に埋め込む(Phase 8 で統合担当が中身を作る)。
//! 窓は画面の真ん中に、狭い窓でもはみ出さない大きさで出す。

use eframe::egui;
use crate::app::State;
use crate::theme::{self, size, space};
use crate::widgets;
use egui::{Align2, CornerRadius, Margin, RichText, Ui, Vec2};

pub const LICENSES: &str = include_str!("../licenses.txt");

pub fn window(st: &mut State, ui: &mut Ui) {
    if !st.about_open {
        return;
    }
    let pal = theme::palette(ui);
    let mut open = true;
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let max = Vec2::new(screen.width() - 24.0, screen.height() - 64.0);
    egui::Window::new("このソフトについて")
        .open(&mut open)
        .collapsible(false)
        .default_size(Vec2::new(560.0, 460.0).min(max))
        .max_size(max)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .show(&ctx, |ui| {
            ui.add_space(space::S);
            ui.label(RichText::new("ProcSight").size(size::HERO).strong());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("版 {}", env!("CARGO_PKG_VERSION"))).color(pal.weak));
                ui.label(RichText::new("・").color(pal.weak));
                ui.label(RichText::new("MIT License").color(pal.weak));
            });
            ui.label(RichText::new(format!("著作者: {}", env!("CARGO_PKG_AUTHORS"))).color(pal.weak));
            ui.add_space(space::M);
            egui::Frame::new()
                .fill(widgets::tint(pal.accent, 28))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(space::M as i8, (space::S + 2.0) as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new("ProcSight はネットにつながりません。履歴を保存しません").color(pal.accent)).wrap());
                });
            ui.add_space(space::S);
            widgets::section(ui, &pal, "依存のライセンス");
            widgets::card(ui, &pal, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("licenses").auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(RichText::new(LICENSES).monospace().size(size::SMALL));
                });
            });
        });
    if !open {
        st.about_open = false;
    }
}
