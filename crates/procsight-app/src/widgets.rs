//! 画面の間で共有する小さな部品(v0.2.0)。色は theme::Palette の役割、余白と文字は theme::space / theme::size だけを使う。
//! 絵文字は使わない(印は図形を描く)。

use crate::theme::{Palette, size, space};
use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};

/// 色を薄めて地に使う(a は 0〜255)
pub fn tint(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// 丸い角の小さな札(状態や件数)。文字の色を薄めた地に、同じ色の文字
pub fn pill(ui: &mut Ui, text: impl Into<String>, fg: Color32) -> Response {
    egui::Frame::new()
        .fill(tint(fg, 38))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| ui.add(egui::Label::new(RichText::new(text.into()).color(fg).size(size::SMALL)).selectable(false)))
        .response
}

/// 色の点と文字(表の状態の列)
pub fn dot_label(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S + 1.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, color);
        ui.add(egui::Label::new(RichText::new(text).color(color)).selectable(false));
    })
    .response
}

/// 縁と地のある面
pub fn card<R>(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(space::L as i8))
        .show(ui, add)
}

/// 見出し(太字)と、右へ伸びる細い線
pub fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(space::S);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        let r = ui.available_rect_before_wrap();
        let y = r.center().y;
        ui.painter().hline(r.left() + space::M..=r.right(), y, Stroke::new(1.0, pal.border));
    });
    ui.add_space(space::XS);
}

/// 弱い文字の名前と、大きな値を縦に
pub fn stat(ui: &mut Ui, pal: &Palette, label: &str, value: impl Into<String>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(label).color(pal.weak).size(size::SMALL));
        ui.label(RichText::new(value.into()).size(size::TITLE));
    });
}

/// 空・読み込み中・取れないの表示。真ん中に題と、弱い文字の説明
pub fn empty_state(ui: &mut Ui, pal: &Palette, title: &str, detail: Option<&str>) {
    ui.add_space(space::XL * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(size::BODY + 1.0));
        if let Some(d) = detail {
            ui.add_space(space::S);
            ui.label(RichText::new(d).color(pal.weak));
        }
    });
}

/// キーの札(「Ctrl」「K」など)
pub fn kbd(ui: &mut Ui, pal: &Palette, key: &str) -> Response {
    egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(Margin::symmetric(5, 0))
        .show(ui, |ui| ui.add(egui::Label::new(RichText::new(key).size(size::SMALL).color(pal.weak)).selectable(false)))
        .response
}

/// 危ない操作の実行のボタン(危険の色の地に白い文字)。既定のボタンにはしない(画面設計書 §9)
pub fn danger_button(pal: &Palette, text: &str) -> egui::Button<'static> {
    // 危険の色そのもの(ダークでは明るい赤)に白い文字だと読みにくいので、地は白い文字が読める暗さまで落とす
    let d = pal.danger;
    let fill = if d.r() as u32 + d.g() as u32 + d.b() as u32 > 400 { Color32::from_rgb(0xB4, 0x23, 0x2C) } else { d };
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE).strong()).fill(fill)
}

/// 主な操作のボタン(強調の色の地)
pub fn primary_button(pal: &Palette, text: &str) -> egui::Button<'static> {
    let fg = if pal.accent.r() as u32 + pal.accent.g() as u32 + pal.accent.b() as u32 > 450 { Color32::BLACK } else { Color32::WHITE };
    egui::Button::new(RichText::new(text.to_string()).color(fg).strong()).fill(pal.accent)
}

/// 上のタブ。選んでいる物は強調の色の下線。badge は右に小さく出す件数
pub fn nav_tab(ui: &mut Ui, pal: &Palette, selected: bool, text: &str, badge: Option<&str>) -> Response {
    let font = FontId::proportional(size::BODY);
    let small = FontId::proportional(size::SMALL);
    let fg = if selected { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let text_g = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let badge_g = badge.map(|b| ui.painter().layout_no_wrap(b.to_string(), small.clone(), pal.weak));
    let pad = Vec2::new(space::L, space::S + 2.0);
    // % の札(CPU)は 1 秒ごとに桁が変わるので、「100%」の幅を取っておき、後ろのタブを動かさない
    let reserve = if badge.is_some_and(|b| b.ends_with('%')) {
        ui.painter().layout_no_wrap("100%".into(), small, pal.weak).size().x
    } else {
        0.0
    };
    let badge_w = badge_g.as_ref().map_or(0.0, |g| g.size().x.max(reserve) + space::S + 2.0);
    let size = Vec2::new(text_g.size().x + badge_w + pad.x * 2.0, text_g.size().y + pad.y * 2.0 + 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() && !selected {
            ui.painter().rect_filled(rect, CornerRadius::same(4), ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let tpos = egui::pos2(rect.left() + pad.x, rect.center().y - text_g.size().y / 2.0 - 1.0);
        let tw = text_g.size().x;
        ui.painter().galley(tpos, text_g, fg);
        if let Some(g) = badge_g {
            let p = egui::pos2(tpos.x + tw + space::S + 2.0, rect.center().y - g.size().y / 2.0 - 1.0);
            ui.painter().galley(p, g, pal.weak);
        }
        if selected {
            let bar = Rect::from_min_max(egui::pos2(rect.left() + pad.x - 2.0, rect.bottom() - 3.0), egui::pos2(rect.right() - pad.x + 2.0, rect.bottom()));
            ui.painter().rect_filled(bar, CornerRadius::same(2), pal.accent);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, text));
    resp
}

/// 0〜1 の横棒(小さなメーター)。幅は width、高さ 6
pub fn meter(ui: &mut Ui, pal: &Palette, frac: f32, width: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    let r = CornerRadius::same(3);
    ui.painter().rect_filled(rect, r, tint(pal.weak, 50));
    let f = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
    if f > 0.0 {
        let mut fill = rect;
        fill.set_width((rect.width() * f).max(3.0));
        ui.painter().rect_filled(fill, r, color);
    }
    resp
}

/// 文字を rect の中の左か右に寄せて描く(表のセルで Label を使わず描くとき)
pub fn text_in(ui: &Ui, rect: Rect, right: bool, text: &str, color: Color32) {
    let (anchor, x) = if right { (Align2::RIGHT_CENTER, rect.right()) } else { (Align2::LEFT_CENTER, rect.left()) };
    ui.painter().text(egui::pos2(x, rect.center().y), anchor, text, FontId::proportional(size::BODY), color);
}

/// 絞り込みの欄(左に虫眼鏡、入っているときだけ右に消すための ×。Esc でも空にする)。S1 と S4 で同じ形にする
pub fn search_box(ui: &mut Ui, pal: &Palette, text: &mut String, hint: &str, width: f32) -> Response {
    let edit = egui::TextEdit::singleline(text).hint_text(hint).margin(Margin { left: 26, right: 22, top: 4, bottom: 4 }).desired_width(width);
    let resp = ui.add(edit);
    let s = Stroke::new(1.4, pal.weak);
    let c = egui::pos2(resp.rect.left() + 13.0, resp.rect.center().y) + Vec2::new(-1.5, -1.5);
    ui.painter().circle_stroke(c, 4.5, s);
    ui.painter().line_segment([c + Vec2::new(3.3, 3.3), c + Vec2::new(7.0, 7.0)], s);
    if !text.is_empty() {
        let r = Rect::from_center_size(egui::pos2(resp.rect.right() - 11.0, resp.rect.center().y), Vec2::splat(16.0));
        let x = ui.interact(r, resp.id.with("clear"), Sense::click()).on_hover_text("絞り込みを消す(Esc)");
        let col = if x.hovered() { ui.visuals().strong_text_color() } else { pal.weak };
        let k = r.shrink(4.5);
        ui.painter().line_segment([k.left_top(), k.right_bottom()], Stroke::new(1.4, col));
        ui.painter().line_segment([k.right_top(), k.left_bottom()], Stroke::new(1.4, col));
        if x.clicked() {
            text.clear();
        }
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        text.clear();
    }
    resp
}
