//! 色を役割の名前(強調・警告・危険・弱い文字・新しい行・終わった行)で持つ。ダークとライトの 2 組(画面設計書 冒頭)。
//! テーマの選択(ダーク・ライト・Windows に合わせる)と文字の大きさを egui に渡す。
//! v0.2.0 から余白・文字の段と使用量の塗り(heat)もここに置く。部品は widgets.rs。

use eframe::egui;
use egui::{Color32, Context, ThemePreference, Ui};
use procsight_core::config::{Accent, HeatFull, HeatStart, Theme};

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
    heat_range(p, frac, HeatStart::Any.frac(), HeatFull::Max.frac())
}

/// 使用量(0〜1)の塗り(v0.4.0 の設定の「塗り始める所」「一番濃くする所」)。start 未満は塗らない。
/// start で一番薄く、full で一番濃い。既定(0.02・1.0)の色は heat と同じ
pub fn heat_range(p: &Palette, frac: f32, start: f32, full: f32) -> Option<Color32> {
    if !frac.is_finite() || frac < start {
        return None;
    }
    // 既定のときは v0.3.0 の色のまま(0.02 から 1.0 へ、0 を起点に混ぜる)
    let t = if start <= HeatStart::Any.frac() { frac / full } else { (frac - start) / (full - start).max(1e-3) }.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Some(Color32::from_rgb(mix(p.heat_lo.r(), p.heat_hi.r()), mix(p.heat_lo.g(), p.heat_hi.g()), mix(p.heat_lo.b(), p.heat_hi.b())))
}

/// 強調の色(ダーク, ライト, 選んだ行のダーク, 選んだ行のライト)。青は v0.3.0 までの色
fn accent_colors(a: Accent) -> [Color32; 4] {
    let c = Color32::from_rgb;
    match a {
        Accent::Blue => [DARK.accent, LIGHT.accent, DARK.selected, LIGHT.selected],
        Accent::Teal => [c(0x3C, 0xD0, 0xC8), c(0x0A, 0x7A, 0x74), c(0x17, 0x4D, 0x4A), c(0xC2, 0xEC, 0xE8)],
        Accent::Green => [c(0x6C, 0xD0, 0x7C), c(0x1E, 0x7A, 0x34), c(0x22, 0x4D, 0x2C), c(0xCC, 0xEE, 0xD4)],
        Accent::Orange => [c(0xFF, 0xA6, 0x4D), c(0xB0, 0x52, 0x00), c(0x5A, 0x3A, 0x14), c(0xFA, 0xE0, 0xC4)],
        Accent::Pink => [c(0xFF, 0x7E, 0xB6), c(0xB0, 0x2A, 0x6E), c(0x5A, 0x25, 0x40), c(0xF8, 0xD2, 0xE4)],
        Accent::Purple => [c(0xB4, 0x8C, 0xFF), c(0x6A, 0x3D, 0xC0), c(0x3E, 0x2D, 0x66), c(0xE2, 0xD6, 0xF8)],
    }
}

/// ダークかライトの組に強調の色を入れる。青なら DARK・LIGHT そのまま
pub fn palette_for(dark: bool, accent: Accent) -> Palette {
    let mut p = if dark { DARK } else { LIGHT };
    if accent != Accent::Blue {
        let [ad, al, sd, sl] = accent_colors(accent);
        p.accent = if dark { ad } else { al };
        p.selected = if dark { sd } else { sl };
        p.graph = p.accent;
        // 橙は 2 本目の線(送信・書き込み・熱い推移)の橙と見分けられないので、2 本目を青にする
        if accent == Accent::Orange {
            p.graph2 = if dark { DARK.graph } else { LIGHT.graph };
        }
    }
    p
}

fn accent_id() -> egui::Id {
    egui::Id::unique("procsight-accent")
}

/// 今の egui の見た目(ダークかライトか)と、apply で入れた強調の色に合う組
pub fn palette(ui: &Ui) -> Palette {
    let accent = ui.ctx().data(|d| d.get_temp::<Accent>(accent_id())).unwrap_or_default();
    palette_for(ui.visuals().dark_mode, accent)
}

pub fn preference(theme: Theme) -> ThemePreference {
    match theme {
        Theme::Dark => ThemePreference::Dark,
        Theme::Light => ThemePreference::Light,
        Theme::System => ThemePreference::System,
    }
}

/// テーマ・文字の大きさ・強調の色を渡す。変わったときだけ呼ぶ
pub fn apply(ctx: &Context, theme: Theme, font_scale: f32, accent: Accent) {
    ctx.set_theme(preference(theme));
    ctx.data_mut(|d| d.insert_temp(accent_id(), accent));
    for dark in [true, false] {
        let p = palette_for(dark, accent);
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

    #[test]
    fn heat_range_moves_both_ends() {
        // 既定は heat と同じ
        for f in [0.0, 0.01, 0.02, 0.3, 0.7, 1.0, 2.0] {
            assert_eq!(heat_range(&DARK, f, 0.02, 1.0), heat(&DARK, f));
        }
        assert_eq!(heat_range(&DARK, 0.3, 0.35, 1.0), None, "塗り始めの手前は塗らない");
        assert_eq!(heat_range(&DARK, 0.35, 0.35, 1.0), Some(DARK.heat_lo), "塗り始めは一番薄い");
        assert_eq!(heat_range(&DARK, 0.5, 0.02, 0.5), Some(DARK.heat_hi), "半分で一番濃い");
        assert_eq!(heat_range(&DARK, 0.75, 0.15, 0.75), Some(DARK.heat_hi));
    }

    #[test]
    fn accent_blue_is_the_old_palette_and_others_differ() {
        for dark in [true, false] {
            let base = if dark { DARK } else { LIGHT };
            let blue = palette_for(dark, Accent::Blue);
            assert_eq!((blue.accent, blue.selected, blue.graph), (base.accent, base.selected, base.graph));
            for a in &Accent::ALL[1..] {
                let p = palette_for(dark, *a);
                assert_ne!(p.accent, base.accent, "{a:?}");
                assert_eq!(p.graph, p.accent);
                assert_eq!(p.danger, base.danger, "強調以外は変えない");
                assert_ne!(p.graph, p.graph2, "2 本の線は別の色");
            }
        }
    }
}
