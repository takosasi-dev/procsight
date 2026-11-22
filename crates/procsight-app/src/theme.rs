//! 色を役割の名前(強調・警告・危険・弱い文字・新しい行・終わった行)で持つ。ダークとライトの 2 組(画面設計書 冒頭)。
//! テーマの選択(ダーク・ライト・Windows に合わせる)と文字の大きさを egui に渡す。
//! v0.2.0 から余白・文字の段と使用量の塗り(heat)もここに置く。部品は widgets.rs。

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
    /// 成功・実行中の印(v0.2.0)
    pub ok: Color32,
    /// カードや浮いた面の地(v0.2.0)
    pub surface: Color32,
    /// 面の縁の線(v0.2.0)
    pub border: Color32,
    /// 使用量の塗り(heat)の弱い端と強い端(v0.2.0)
    pub heat_lo: Color32,
    pub heat_hi: Color32,
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
    ok: Color32::from_rgb(0x5F, 0xD0, 0x8A),
    surface: Color32::from_rgb(0x24, 0x27, 0x2B),
    border: Color32::from_rgb(0x3A, 0x3F, 0x46),
    heat_lo: Color32::from_rgb(0x33, 0x2E, 0x1F),
    heat_hi: Color32::from_rgb(0x8C, 0x3F, 0x14),
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
    ok: Color32::from_rgb(0x1E, 0x7E, 0x45),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xD6, 0xDA, 0xDF),
    heat_lo: Color32::from_rgb(0xFF, 0xF4, 0xCC),
    heat_hi: Color32::from_rgb(0xFF, 0xA8, 0x5C),
};

/// 余白の段(ポイント)。画面の間を同じ物差しでそろえる(v0.2.0)
pub mod space {
    pub const XS: f32 = 2.0;
    pub const S: f32 = 4.0;
    pub const M: f32 = 8.0;
    pub const L: f32 = 12.0;
    pub const XL: f32 = 20.0;
}

/// 文字の大きさの段(ポイント。egui の既定の本文は 14)
pub mod size {
    pub const SMALL: f32 = 12.0;
    pub const BODY: f32 = 14.0;
    pub const TITLE: f32 = 18.0;
    pub const HERO: f32 = 26.0;
}

/// 使用量(0〜1)の塗り。0.02 未満は塗らない。間は heat_lo → heat_hi を直線で混ぜる
pub fn heat(p: &Palette, frac: f32) -> Option<Color32> {
    if !frac.is_finite() || frac < 0.02 {
        return None;
    }
    let t = frac.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Some(Color32::from_rgb(mix(p.heat_lo.r(), p.heat_hi.r()), mix(p.heat_lo.g(), p.heat_hi.g()), mix(p.heat_lo.b(), p.heat_hi.b())))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_ramp_ends_and_skips_tiny() {
        assert_eq!(heat(&DARK, 0.0), None);
        assert_eq!(heat(&DARK, 0.01), None);
        assert_eq!(heat(&DARK, f32::NAN), None);
        assert_eq!(heat(&DARK, 1.0), Some(DARK.heat_hi));
        assert_eq!(heat(&LIGHT, 5.0), Some(LIGHT.heat_hi));
        let mid = heat(&LIGHT, 0.5).unwrap();
        assert!(mid.g() < LIGHT.heat_lo.g() && mid.g() > LIGHT.heat_hi.g());
    }
}
