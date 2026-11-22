//! 色を役割の名前(強調・警告・危険・弱い文字・新しい行・終わった行)で持つ。ダークとライトの 2 組(画面設計書 冒頭)。
//! テーマの選択(ダーク・ライト・Windows に合わせる)と文字の大きさを egui に渡す。

use eframe::egui;
use egui::{Color32, Context, ThemePreference, Ui};
use procsight_core::config::Theme;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub weak: Color32,
    pub new_row: Color32,
    pub gone_row: Color32,
    pub selected: Color32,
    pub band_ok: Color32,
    pub band_warn: Color32,
    pub band_danger: Color32,
    pub graph: Color32,
    pub graph2: Color32,
}

pub const DARK: Palette = Palette {
    accent: Color32::from_rgb(0x5C, 0xB8, 0xFF),
    warn: Color32::from_rgb(0xF0, 0xB4, 0x3C),
    danger: Color32::from_rgb(0xFF, 0x6B, 0x6B),
    weak: Color32::from_rgb(0x8C, 0x96, 0xA0),
    new_row: Color32::from_rgb(0x1F, 0x4A, 0x2E),
    gone_row: Color32::from_rgb(0x5A, 0x24, 0x28),
    selected: Color32::from_rgb(0x1F, 0x4E, 0x7A),
    band_ok: Color32::from_rgb(0x17, 0x3A, 0x5C),
    band_warn: Color32::from_rgb(0x4D, 0x3B, 0x12),
    band_danger: Color32::from_rgb(0x5C, 0x1E, 0x22),
    graph: Color32::from_rgb(0x5C, 0xB8, 0xFF),
    graph2: Color32::from_rgb(0xF0, 0x8C, 0x50),
};

pub const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(0x0B, 0x5C, 0xAD),
    warn: Color32::from_rgb(0x9A, 0x5B, 0x00),
    danger: Color32::from_rgb(0xC0, 0x1C, 0x28),
    weak: Color32::from_rgb(0x6A, 0x72, 0x7A),
    new_row: Color32::from_rgb(0xD4, 0xF2, 0xDC),
    gone_row: Color32::from_rgb(0xF8, 0xD7, 0xDA),
    selected: Color32::from_rgb(0xC6, 0xDF, 0xF7),
    band_ok: Color32::from_rgb(0xDD, 0xEB, 0xF8),
    band_warn: Color32::from_rgb(0xFD, 0xF0, 0xD0),
    band_danger: Color32::from_rgb(0xFB, 0xDE, 0xE0),
    graph: Color32::from_rgb(0x0B, 0x6C, 0xC4),
    graph2: Color32::from_rgb(0xD0, 0x60, 0x20),
};

/// 今の egui の見た目(ダークかライトか)に合う組
pub fn palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

pub fn preference(theme: Theme) -> ThemePreference {
    match theme {
        Theme::Dark => ThemePreference::Dark,
        Theme::Light => ThemePreference::Light,
        Theme::System => ThemePreference::System,
    }
}

/// テーマと文字の大きさを渡す。変わったときだけ呼ぶ
pub fn apply(ctx: &Context, theme: Theme, font_scale: f32) {
    ctx.set_theme(preference(theme));
    for (dark, p) in [(true, DARK), (false, LIGHT)] {
        let t = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(t, |s| {
            s.visuals.selection.bg_fill = p.selected;
            s.visuals.hyperlink_color = p.accent;
            s.visuals.warn_fg_color = p.warn;
            s.visuals.error_fg_color = p.danger;
            s.interaction.selectable_labels = false;
        });
    }
    ctx.set_zoom_factor(font_scale.clamp(0.8, 1.6));
}
