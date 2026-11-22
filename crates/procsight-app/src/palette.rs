//! コマンド パレット(Ctrl+K)とキーボードのショートカットの一覧(F1)。
//! パレットはタブを移る・窓を開く・見た目を変える・行へ移るだけで、危ない操作(終了・一時停止・サービスの停止・開始の種類・置き換え)は積まない(INV-5)。
//! 候補の文字は Snapshot の seq とサービスの一覧が変わったときだけ作り、照合は入力が変わったときだけする。

use crate::app::{Load, State, Tab, interval_label};
use crate::dialogs::Dialog;
use crate::services_view::StateFilter;
use crate::theme::{self, Palette, size, space};
use crate::widgets;
use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, CornerRadius, Event, FontId, Id, Key, Layout, Margin, Modifiers, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};
use procsight_core::config::{INTERVALS_MS, Theme, View};
use procsight_core::{ProcId, format};

/// 一覧に出す最大の件数
const MAX_HITS: usize = 50;
/// 面の幅(狭い窓では窓の幅 − 余白)
const WIDTH: f32 = 560.0;
const ROW_H: f32 = 30.0;
/// PageUp / PageDown で動かす行の数
const PAGE: i32 = 8;
/// 補助の文字(PID・サービスの名前・英語の別名)で合ったときに引く点。題で合った物を先に出す
const SUB_PENALTY: i32 = 500;

#[derive(Default)]
pub struct PaletteView {
    /// パレットを開いている
    pub open: bool,
    /// ショートカットの一覧を開いている
    pub help_open: bool,
    pub query: String,
    /// 選んでいる結果の位置
    sel: usize,
    /// 選んだ行を見える位置へ送る(キーで動かしたフレームだけ)
    scroll_sel: bool,
    /// 前のフレームで開いていた(開いた・閉じた瞬間を見る)
    was_open: bool,
    help_was_open: bool,
    /// Enter を受けた。結果を作り直した後で実行する
    run_pending: bool,
    close_pending: bool,
    /// プロセスとサービスの候補(seq とサービスの一覧が変わったときだけ作る)
    pool: Vec<Item>,
    pool_key: Option<PoolKey>,
    hits: Vec<Hit>,
    /// 一致した数(MAX_HITS で切る前)
    total: usize,
    hits_key: Option<HitsKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Tab,
    Op,
    Proc,
    Service,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Tab => "タブ",
            Kind::Op => "操作",
            Kind::Proc => "プロセス",
            Kind::Service => "サービス",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Tab(Tab),
    Settings,
    Locks,
    Pause(bool),
    Interval(u64),
    View(View),
    Theme(Theme),
    RestartElevated,
    About,
    Shortcuts,
    Mini,
    AlwaysOnTop,
    ExportCsv,
    Proc(ProcId),
    Service(String),
}

/// 照合に使う文字。元の字(語の頭を見る)と小文字にした字を、同じ並びで持つ
#[derive(Clone, Debug, Default)]
pub(crate) struct Text {
    chars: Vec<char>,
    lower: Vec<char>,
}

impl Text {
    pub(crate) fn new(s: impl Into<String>) -> Text {
        let s: String = s.into();
        let chars: Vec<char> = s.chars().collect();
        let lower = chars.iter().map(|c| lower(*c)).collect();
        Text { chars, lower }
    }
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[derive(Clone, Debug)]
struct Item {
    kind: Kind,
    title: Text,
    /// 題の右に弱い色で出す(PID・サービスの名前)
    sub: Text,
    /// 出さないが照合に使う別名(英語の言い方)
    alias: Text,
    /// ショートカット(widgets::kbd で添える)
    keys: &'static str,
    /// 今の設定と同じ
    current: bool,
    cmd: Cmd,
}

impl Item {
    fn new(kind: Kind, title: impl Into<String>, cmd: Cmd) -> Item {
        Item { kind, title: Text::new(title), sub: Text::default(), alias: Text::default(), keys: "", current: false, cmd }
    }
    fn sub(mut self, s: impl Into<String>) -> Item {
        self.sub = Text::new(s);
        self
    }
    fn alias(mut self, s: &str) -> Item {
        self.alias = Text::new(s);
        self
    }
    fn keys(mut self, k: &'static str) -> Item {
        self.keys = k;
        self
    }
    fn current(mut self, on: bool) -> Item {
        self.current = on;
        self
    }
}

#[derive(Clone, Debug)]
struct Hit {
    item: Item,
    /// 合った字の位置(題か補助の文字の char の番号)
    title_marks: Vec<usize>,
    sub_marks: Vec<usize>,
}

/// (Snapshot の seq, サービスの一覧の先頭の番地と件数)
type PoolKey = (u64, usize, usize);

#[derive(Clone, Debug, PartialEq)]
struct HitsKey {
    query: String,
    pool: PoolKey,
    tab: Tab,
    paused: bool,
    elevated: bool,
    view: View,
    theme: Theme,
    interval: u64,
    always_on_top: bool,
}

fn word_start(t: &[char], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let (p, c) = (t[i - 1], t[i]);
    p.is_whitespace()
        || "._-/:()[]・、:(「".contains(p)
        || (p.is_ascii_lowercase() && c.is_ascii_uppercase())
        || (p.is_ascii() != c.is_ascii())
}

/// 照合の点数。合わなければ None。大きいほど上で、先頭一致(4 千台)> 語の頭からの続いた一致(3 千台)> どこかの続いた一致(2 千台)> 飛び飛び(千台)。
/// 同じ段の中では、余った字が少ないほど・合った位置が前ほど上。q は空白を除いて小文字にした字。戻りの 2 つめは合った字の位置
pub(crate) fn score(q: &[char], text: &Text) -> Option<(i32, Vec<usize>)> {
    let (t, lc) = (&text.chars, &text.lower);
    let n = q.len();
    if n == 0 {
        return Some((0, Vec::new()));
    }
    if n > lc.len() {
        return None;
    }
    let extra = (lc.len() - n).min(99) as i32;
    let mut best: Option<(i32, usize)> = None;
    for i in 0..=lc.len() - n {
        if lc[i..i + n] == *q {
            let tier = if i == 0 {
                4
            } else if word_start(t, i) {
                3
            } else {
                2
            };
            let s = tier * 1000 - extra - (i.min(99) as i32);
            if best.is_none_or(|(b, _)| s > b) {
                best = Some((s, i));
            }
            if tier == 4 {
                break;
            }
        }
    }
    if let Some((s, i)) = best {
        return Some((s, (i..i + n).collect()));
    }
    // ponytail: 左から貪欲に拾う。語の頭を選び直す最適化(DP)はしない。候補の題が短いので順位の差は小さい
    let mut marks = Vec::with_capacity(n);
    for (i, c) in lc.iter().enumerate() {
        if marks.len() < n && *c == q[marks.len()] {
            marks.push(i);
        }
    }
    if marks.len() < n {
        return None;
    }
    let heads = marks.iter().filter(|&&i| word_start(t, i)).count() as i32;
    let runs = marks.windows(2).filter(|w| w[1] == w[0] + 1).count() as i32;
    Some((1000 + (heads * 60 + runs * 30).min(800) - extra - (marks[0].min(99) as i32), marks))
}

/// 入力を照合の字にする(空白を除き、小文字に)
pub(crate) fn query_chars(query: &str) -> Vec<char> {
    query.chars().filter(|c| !c.is_whitespace()).map(lower).collect()
}

/// (点数, 題で合った字の位置, 補助の文字で合った字の位置)
type Scored = (i32, Vec<usize>, Vec<usize>);

/// 1 つの候補の点数(題・補助の文字・別名のうち良い物)。空の入力ではタブと操作だけ
fn score_item(q: &[char], it: &Item) -> Option<Scored> {
    if q.is_empty() {
        return matches!(it.kind, Kind::Tab | Kind::Op).then(|| (0, Vec::new(), Vec::new()));
    }
    let mut best: Option<Scored> = score(q, &it.title).map(|(s, m)| (s, m, Vec::new()));
    if let Some((s, m)) = score(q, &it.sub) {
        let s = s - SUB_PENALTY;
        if best.as_ref().is_none_or(|b| s > b.0) {
            best = Some((s, Vec::new(), m));
        }
    }
    if let Some((s, _)) = score(q, &it.alias) {
        let s = s - SUB_PENALTY;
        if best.as_ref().is_none_or(|b| s > b.0) {
            best = Some((s, Vec::new(), Vec::new()));
        }
    }
    best
}

/// 並べた結果(最大 MAX_HITS 件)と一致した数。同点なら タブ > 操作 > プロセス > サービス、その中は元の順
fn rank(q: &[char], fixed: &[Item], pool: &[Item]) -> (Vec<Hit>, usize) {
    let mut found: Vec<(Scored, Kind, usize)> = Vec::new();
    for (i, it) in fixed.iter().chain(pool).enumerate() {
        if let Some(sc) = score_item(q, it) {
            found.push((sc, it.kind, i));
        }
    }
    found.sort_by(|a, b| b.0.0.cmp(&a.0.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let total = found.len();
    found.truncate(MAX_HITS);
    let item = |i: usize| if i < fixed.len() { &fixed[i] } else { &pool[i - fixed.len()] };
    let hits = found.into_iter().map(|((_, title_marks, sub_marks), _, i)| Hit { item: item(i).clone(), title_marks, sub_marks }).collect();
    (hits, total)
}

fn pool_key(st: &State) -> PoolKey {
    match &st.services.list {
        Load::Ready(v) => (st.snap.seq, v.as_ptr() as usize, v.len()),
        _ => (st.snap.seq, 0, 0),
    }
}

/// タブと操作(状態で変わるので、照合し直すたびに作る。20 件ほど)
fn fixed_items(st: &State) -> Vec<Item> {
    let mut v = Vec::new();
    let tab_keys = ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4"];
    let tab_alias = ["processes", "performance", "startup", "services"];
    for ((t, k), a) in Tab::ALL.into_iter().zip(tab_keys).zip(tab_alias) {
        v.push(Item::new(Kind::Tab, t.label(), Cmd::Tab(t)).keys(k).alias(a).current(st.tab == t));
    }
    let op = |title: &str, cmd: Cmd| Item::new(Kind::Op, title, cmd);
    v.push(op("設定を開く", Cmd::Settings).alias("settings preferences"));
    v.push(op("ファイルを掴んでいるプロセスを探す", Cmd::Locks).alias("lock file handle"));
    if st.paused {
        v.push(op("更新を再開する", Cmd::Pause(false)).alias("resume refresh"));
    } else {
        v.push(op("更新を一時停止する", Cmd::Pause(true)).alias("pause refresh"));
    }
    for ms in INTERVALS_MS {
        v.push(op(&format!("更新間隔: {}", interval_label(ms)), Cmd::Interval(ms)).alias("interval refresh").current(st.cfg.interval_ms == ms));
    }
    v.push(op("ツリーで表示", Cmd::View(View::Tree)).alias("tree view").current(st.cfg.view == View::Tree));
    v.push(op("一覧で表示", Cmd::View(View::Flat)).alias("list flat view").current(st.cfg.view == View::Flat));
    for (t, name, alias) in [(Theme::Dark, "ダーク", "theme dark"), (Theme::Light, "ライト", "theme light"), (Theme::System, "Windows に合わせる", "theme system")] {
        v.push(op(&format!("テーマ: {name}"), Cmd::Theme(t)).alias(alias).current(st.cfg.theme == t));
    }
    if !st.env.elevated {
        v.push(op("管理者で再起動…", Cmd::RestartElevated).alias("admin elevate restart"));
    }
    v.push(op("ミニ表示", Cmd::Mini).keys("Ctrl+M").alias("mini compact small window"));
    v.push(op("常に手前に表示", Cmd::AlwaysOnTop).alias("always on top pin topmost").current(st.cfg.always_on_top));
    v.push(op("一覧を CSV で保存…", Cmd::ExportCsv).alias("csv export save"));
    v.push(op("このソフトについて", Cmd::About).alias("about version license"));
    v.push(op("キーボードのショートカット", Cmd::Shortcuts).keys("F1").alias("keyboard shortcuts help"));
    v
}

/// 終わっていないプロセスと、読み込み済みのサービス
fn pool_items(st: &State) -> Vec<Item> {
    let mut v: Vec<Item> = st
        .snap
        .rows
        .iter()
        .filter(|r| r.is_gone == 0)
        .map(|r| Item::new(Kind::Proc, crate::procs::display_name(st, r), Cmd::Proc(r.id)).sub(format!("PID {}", r.id.pid)))
        .collect();
    if let Load::Ready(list) = &st.services.list {
        v.extend(list.iter().map(|s| {
            let title = if s.display_name.is_empty() { &s.name } else { &s.display_name };
            Item::new(Kind::Service, title.as_str(), Cmd::Service(s.name.clone())).sub(s.name.as_str())
        }));
    }
    v
}

/// 入力・Snapshot・サービス・状態のどれかが変わったときだけ照合し直す
fn refresh_hits(pv: &mut PaletteView, st: &State) {
    let pk = pool_key(st);
    if pv.pool_key != Some(pk) {
        pv.pool = pool_items(st);
        pv.pool_key = Some(pk);
    }
    let key = HitsKey {
        query: pv.query.clone(),
        pool: pk,
        tab: st.tab,
        paused: st.paused,
        elevated: st.env.elevated,
        view: st.cfg.view,
        theme: st.cfg.theme,
        interval: st.cfg.interval_ms,
        always_on_top: st.cfg.always_on_top,
    };
    if pv.hits_key.as_ref() == Some(&key) {
        return;
    }
    let query_changed = pv.hits_key.as_ref().is_none_or(|k| k.query != key.query);
    let before = pv.hits.get(pv.sel).map(|h| h.item.cmd.clone());
    let (hits, total) = rank(&query_chars(&pv.query), &fixed_items(st), &pv.pool);
    pv.hits = hits;
    pv.total = total;
    if query_changed {
        pv.sel = 0;
        pv.scroll_sel = true;
    } else if let Some(i) = before.and_then(|c| pv.hits.iter().position(|h| h.item.cmd == c)) {
        // 更新で並びが変わっても、選んでいた物を選んだままにする(Enter が隣の行に当たらない)
        pv.sel = i;
    }
    pv.sel = pv.sel.min(pv.hits.len().saturating_sub(1));
    pv.hits_key = Some(key);
}

fn edit_id() -> Id {
    Id::unique("procsight-palette-query")
}

/// 開いた・閉じた瞬間の後始末。consume_keys と window の両方から呼ぶ(メニューで開いたときは window で拾う)
fn sync_open(st: &mut State, ui: &Ui) {
    let pv = &mut st.palette;
    if pv.open && !pv.was_open {
        pv.was_open = true;
        pv.sel = 0;
        pv.hits_key = None;
        pv.run_pending = false;
        pv.close_pending = false;
        // サービスを一度も読んでいなければ読む(読むだけの Job。探せるようにする)
        if st.services.list.is_not_asked() {
            st.job_load_services();
        }
    } else if !pv.open && pv.was_open {
        pv.was_open = false;
        pv.pool = Vec::new();
        pv.pool_key = None;
        pv.hits = Vec::new();
        pv.hits_key = None;
        ui.memory_mut(|m| m.surrender_focus(edit_id()));
    }
}

/// 入力欄にフォーカスを置く。毎フレーム呼ぶと IME の変換が切れるので、無いときだけ
fn keep_focus(ui: &Ui) {
    let id = edit_id();
    if !ui.memory(|m| m.has_focus(id)) {
        ui.memory_mut(|m| m.request_focus(id));
    }
}

/// パレットとショートカットの一覧を描く(開いていなければ何もしない)。窓の中身を描いた後に呼ばれる
pub fn window(st: &mut State, ui: &mut Ui) {
    // パレットを開いたまま F1 を押したら、一覧を前に出すためにパレットを閉じる
    if st.palette.help_open && !st.palette.help_was_open && st.palette.open {
        st.palette.open = false;
    }
    st.palette.help_was_open = st.palette.help_open;
    sync_open(st, ui);
    help_window(st, ui);
    // 確認の窓が出ている間は描かない(Esc・Enter を横取りしない)
    if st.palette.open && st.dialog.is_none() {
        keep_focus(ui);
        palette_window(st, ui);
    }
}

/// パレットが開いている間、表やタブより先にキー(↑↓・PageUp/PageDown・Enter・Esc)を受け取る。窓の中身を描く前に呼ばれる。
/// Delete・Home/End・Ctrl+C は入力欄にフォーカスを置くことで表に渡さない(procs の keyboard はフォーカスがあると動かない)
pub fn consume_keys(st: &mut State, ui: &mut Ui) {
    sync_open(st, ui);
    if st.dialog.is_some() {
        return;
    }
    if !st.palette.open {
        if st.palette.help_open && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            st.palette.help_open = false;
        }
        return;
    }
    keep_focus(ui);
    let pv = &mut st.palette;
    let n = pv.hits.len();
    ui.input_mut(|i| {
        // IME の変換を確定した Enter で実行しない(同じフレームに IME の知らせがあれば見送る)
        let ime = i.events.iter().any(|e| matches!(e, Event::Ime(_)));
        for (k, d) in [(Key::ArrowUp, -1), (Key::ArrowDown, 1), (Key::PageUp, -PAGE), (Key::PageDown, PAGE)] {
            while i.consume_key(Modifiers::NONE, k) {
                pv.sel = step(pv.sel, d, n);
                pv.scroll_sel = true;
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) && !ime {
            pv.run_pending = true;
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            pv.close_pending = true;
        }
    });
}

/// 選ぶ位置を d 動かす。1 つずつのときは端で反対の端へ回る
fn step(sel: usize, d: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let last = n as i32 - 1;
    let cur = sel.min(n - 1) as i32;
    let next = match d {
        -1 if cur == 0 => last,
        1 if cur == last => 0,
        _ => (cur + d).clamp(0, last),
    };
    next as usize
}

fn palette_window(st: &mut State, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let pal = theme::palette(ui);
    let screen = ctx.content_rect();
    let w = WIDTH.min(screen.width() - space::XL * 2.0).max(240.0);
    let top = (screen.height() / 5.0).round();
    // 入力欄と下の案内と縁の分(~90)を引いた残りに収める
    let list_max = (screen.height() - top - space::XL - 90.0).clamp(ROW_H * 3.0, ROW_H * 12.0);
    let dark = ui.visuals().dark_mode;
    let id = Id::unique("procsight-palette");
    let frame = egui::Frame::new()
        .fill(pal.surface)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(8))
        .shadow(ui.visuals().popup_shadow);
    let mut pv = std::mem::take(&mut st.palette);
    let mut clicked = None;
    let modal = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, Vec2::new(0.0, top)))
        .backdrop_color(Color32::from_black_alpha(if dark { 110 } else { 60 }))
        .frame(frame)
        .show(&ctx, |ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            input_row(ui, &pal, &mut pv);
            refresh_hits(&mut pv, st);
            divider(ui, &pal);
            clicked = results(ui, &pal, &mut pv, list_max);
            divider(ui, &pal);
            footer(ui, &pal, &pv);
        });
    let run = if pv.run_pending && !pv.hits.is_empty() { Some(pv.sel) } else { clicked };
    let close = pv.close_pending || modal.backdrop_response.clicked();
    pv.run_pending = false;
    pv.close_pending = false;
    let cmd = run.and_then(|i| pv.hits.get(i)).map(|h| h.item.cmd.clone());
    st.palette = pv;
    if cmd.is_some() || close {
        st.palette.open = false;
        sync_open(st, ui);
    }
    if let Some(c) = cmd {
        execute(st, c);
    }
}

fn input_row(ui: &mut Ui, pal: &Palette, pv: &mut PaletteView) {
    egui::Frame::new().inner_margin(Margin::symmetric(space::L as i8, space::M as i8 + 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            magnifier(ui, r, pal.weak);
            ui.add_space(space::M);
            let edit = egui::TextEdit::singleline(&mut pv.query)
                .id(edit_id())
                .hint_text(RichText::new("タブ・操作・プロセス・サービスを探す").size(size::TITLE))
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .font(FontId::proportional(size::TITLE))
                .desired_width(f32::INFINITY);
            ui.add(edit);
        });
    });
}

/// 虫眼鏡の印(絵文字は使わない)
fn magnifier(ui: &Ui, r: Rect, color: Color32) {
    let s = Stroke::new(1.6, color);
    let c = r.center() + Vec2::new(-2.0, -2.0);
    ui.painter().circle_stroke(c, 5.5, s);
    ui.painter().line_segment([c + Vec2::splat(4.0), c + Vec2::splat(8.5)], s);
}

fn divider(ui: &mut Ui, pal: &Palette) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, pal.border));
}

/// 結果の一覧。押された行の位置を返す
fn results(ui: &mut Ui, pal: &Palette, pv: &mut PaletteView, max_h: f32) -> Option<usize> {
    if pv.hits.is_empty() {
        egui::Frame::new().inner_margin(Margin::same(space::L as i8)).show(ui, |ui| {
            ui.vertical_centered(|ui| ui.label(RichText::new("一致する物がありません").color(pal.weak)));
        });
        return None;
    }
    let mut clicked = None;
    let scroll = std::mem::take(&mut pv.scroll_sel);
    egui::ScrollArea::vertical().id_salt("palette-list").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(space::S as i8)).show(ui, |ui| {
            for (i, h) in pv.hits.iter().enumerate() {
                let resp = hit_row(ui, pal, h, i == pv.sel);
                if i == pv.sel && scroll {
                    ui.scroll_to_rect(resp.rect, None);
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}

fn hit_row(ui: &mut Ui, pal: &Palette, h: &Hit, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    // 読み上げ(と画面のテスト)には、題を押せる項目として渡す
    let name: String = h.item.title.chars.iter().collect();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(4), pal.selected);
        let bar = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 6.0), Vec2::new(3.0, rect.height() - 12.0));
        p.rect_filled(bar, CornerRadius::same(2), pal.accent);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    // 札が行の高さまで伸びないよう、中身は 1 行分の高さに収める
    let inner = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - space::L * 2.0, 20.0));
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)));
    row.spacing_mut().item_spacing.x = space::S;
    widgets::pill(&mut row, h.item.kind.label(), pal.weak);
    if !h.item.keys.is_empty() {
        row.add_space(space::S);
        keys_ui(&mut row, pal, h.item.keys, true);
    }
    row.with_layout(Layout::left_to_right(Align::Center), |ui| {
        let fg = if selected { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
        ui.add(egui::Label::new(title_job(pal, h, fg)).truncate().selectable(false));
    });
    resp
}

/// 題(合った字は強調の色)・補助の文字(弱い色)・「使用中」を 1 行に
fn title_job(pal: &Palette, h: &Hit, fg: Color32) -> LayoutJob {
    let body = FontId::proportional(size::BODY);
    let mut job = LayoutJob::default();
    let push = |job: &mut LayoutJob, chars: &[char], marks: &[usize], color: Color32, lead: f32| {
        let mut i = 0;
        while i < chars.len() {
            let hit = marks.contains(&i);
            let start = i;
            while i < chars.len() && marks.contains(&i) == hit {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let c = if hit { pal.accent } else { color };
            let lead = if start == 0 { lead } else { 0.0 };
            job.append(&s, lead, TextFormat { font_id: body.clone(), color: c, valign: Align::Center, ..Default::default() });
        }
    };
    push(&mut job, &h.item.title.chars, &h.title_marks, fg, 0.0);
    push(&mut job, &h.item.sub.chars, &h.sub_marks, pal.weak, space::M);
    if h.item.current {
        let f = TextFormat { font_id: FontId::proportional(size::SMALL), color: pal.ok, valign: Align::Center, ..Default::default() };
        job.append(if h.item.kind == Kind::Tab { "表示中" } else { "使用中" }, space::M, f);
    }
    job
}

/// キーの組を札で描く。「Ctrl+K」は札を「+」でつなぎ、空白で区切った「〜」「/」は弱い字で出す。
/// rev は右から左へ並べる場所(一覧の行の右端)で使う。
fn keys_ui(ui: &mut Ui, pal: &Palette, spec: &str, rev: bool) {
    let mut parts: Vec<(bool, &str)> = Vec::new();
    for token in spec.split(' ') {
        if token == "〜" || token == "/" {
            parts.push((false, token));
            continue;
        }
        for (j, k) in token.split('+').enumerate() {
            if j > 0 {
                parts.push((false, "+"));
            }
            parts.push((true, k));
        }
    }
    if rev {
        parts.reverse();
    }
    let gap = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = space::XS + 1.0;
    for (is_key, s) in parts {
        if is_key {
            widgets::kbd(ui, pal, s);
        } else {
            ui.add(egui::Label::new(RichText::new(s).color(pal.weak).size(size::SMALL)).selectable(false));
        }
    }
    ui.spacing_mut().item_spacing.x = gap;
}

fn footer(ui: &mut Ui, pal: &Palette, pv: &PaletteView) {
    egui::Frame::new().inner_margin(Margin::symmetric(space::L as i8, space::S as i8 + 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::S;
            let hint = |ui: &mut Ui, keys: &str, what: &str| {
                keys_ui(ui, pal, keys, false);
                ui.add(egui::Label::new(RichText::new(what).color(pal.weak).size(size::SMALL)).selectable(false));
                ui.add_space(space::M);
            };
            hint(ui, "↑ ↓", "選ぶ");
            hint(ui, "Enter", "実行");
            hint(ui, "Esc", "閉じる");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let n = if pv.total > pv.hits.len() {
                    format!("{} 件中 {} 件", format::thousands(pv.total as u64), pv.hits.len())
                } else {
                    format!("{} 件", pv.hits.len())
                };
                ui.add(egui::Label::new(RichText::new(n).color(pal.weak).size(size::SMALL)).selectable(false));
            });
        });
    });
}

/// 選んだ候補を実行する。どれも移る・開く・見た目を変えるだけ(確認の窓が要る操作は「管理者で再起動」の窓を出すまで)
fn execute(st: &mut State, cmd: Cmd) {
    match cmd {
        Cmd::Tab(t) => st.tab = t,
        Cmd::Settings => crate::settings::open(st),
        Cmd::Locks => st.locks.open = true,
        Cmd::Pause(p) => {
            if st.paused != p {
                st.set_paused(p);
            }
        }
        Cmd::Interval(ms) => {
            if st.cfg.interval_ms != ms {
                st.set_interval(ms);
            }
        }
        Cmd::View(v) => {
            if st.cfg.view != v {
                st.cfg.view = v;
                st.save_config();
            }
        }
        Cmd::Theme(t) => {
            if st.cfg.theme != t {
                st.cfg.theme = t;
                st.save_config();
            }
        }
        Cmd::RestartElevated => {
            if !st.env.elevated {
                st.dialog = Some(Dialog::RestartElevated { opened: false });
            }
        }
        Cmd::About => st.about_open = true,
        Cmd::Mini => st.set_mini(true),
        Cmd::AlwaysOnTop => st.set_always_on_top(!st.cfg.always_on_top),
        Cmd::ExportCsv => crate::export::request(st),
        Cmd::Shortcuts => {
            st.palette.help_open = true;
            st.palette.help_was_open = true;
        }
        Cmd::Proc(id) => {
            st.procs.scroll_to = Some(id);
            st.tab = Tab::Processes;
        }
        Cmd::Service(name) => {
            // 絞り込みで隠れないように外してから選ぶ
            st.services.filter.clear();
            st.services.state_filter = StateFilter::All;
            st.services.scroll_to = Some(name.clone());
            st.services.selected = Some(name);
            st.tab = Tab::Services;
        }
    }
}

/// ショートカットの一覧(画面設計書 §8 と Ctrl+K・F1)。場所ごとに (見出し, [(キー, 動き)])
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "どこでも",
        &[
            ("Ctrl+K", "コマンド パレットを開く・閉じる"),
            ("F1", "キーボードのショートカット(この一覧)"),
            ("Ctrl+1 〜 Ctrl+4", "タブの切り替え"),
            ("Ctrl+M", "ミニ表示に切り替える・戻す"),
        ],
    ),
    ("プロセスの表", &[("↑ / ↓", "選ぶ行を動かす"), ("PageUp / PageDown", "選ぶ行を 1 ページ分動かす"), ("Home / End", "先頭・末尾の行を選ぶ")]),
    (
        "プロセス",
        &[
            ("Enter", "詳細パネルを開く"),
            ("Delete", "終了(確認の窓を出します)"),
            ("Ctrl+C", "選んだ行をコピー"),
            ("← / →", "ツリーを畳む・開く"),
        ],
    ),
    ("プロセス・サービスの絞り込み", &[("Ctrl+F", "絞り込みの欄へ"), ("Esc", "絞り込みの欄を空にする")]),
    ("スタートアップ・サービス", &[("F5", "取り直す")]),
    ("確認の窓", &[("Esc", "「やめる」と同じ")]),
    ("コマンド パレット", &[("↑ / ↓", "選ぶ"), ("PageUp / PageDown", "大きく動かす"), ("Enter", "実行する"), ("Esc", "閉じる")]),
];

fn help_window(st: &mut State, ui: &mut Ui) {
    if !st.palette.help_open {
        return;
    }
    let pal = theme::palette(ui);
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let mut open = true;
    egui::Window::new("キーボードのショートカット")
        .id(Id::unique("procsight-shortcuts"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(460.0)
        .show(&ctx, |ui| {
            let max_h = (screen.height() - 140.0).max(160.0);
            egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h).show(ui, |ui| {
                for (title, rows) in SHORTCUTS {
                    widgets::section(ui, &pal, title);
                    egui::Grid::new(("shortcuts", *title)).num_columns(2).min_col_width(170.0).spacing([space::L, space::S + 2.0]).show(ui, |ui| {
                        for (keys, what) in *rows {
                            ui.horizontal(|ui| keys_ui(ui, &pal, keys, false));
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
                    ui.add_space(space::S);
                }
            });
        });
    if !open {
        st.palette.help_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, text: &str) -> Option<i32> {
        score(&query_chars(q), &Text::new(text)).map(|(s, _)| s)
    }

    #[test]
    fn tiers_prefix_word_contiguous_scattered() {
        let prefix = s("note", "notepad.exe").unwrap();
        let word = s("note", "my notes").unwrap();
        let inside = s("note", "keynote").unwrap();
        let scattered = s("note", "n_o_t_e").unwrap();
        assert!(prefix > word && word > inside && inside > scattered, "{prefix} {word} {inside} {scattered}");
        assert_eq!(s("note", "pad"), None);
        assert_eq!(s("xyz", "notepad.exe"), None);
    }

    #[test]
    fn case_insensitive_and_spaces_ignored() {
        assert!(s("NOTE", "notepad.exe").unwrap() > 3900);
        assert!(s("Note", "NOTEPAD.EXE").unwrap() > 3900);
        assert!(s("windows search", "WindowsSearch").unwrap() > 3900);
    }

    #[test]
    fn exact_beats_longer_and_marks_are_char_positions() {
        assert!(s("サービス", "サービス").unwrap() > s("サービス", "サービスの一覧").unwrap());
        let (_, m) = score(&query_chars("ps"), &Text::new("ProcSight")).unwrap();
        assert_eq!(m, vec![0, 4]);
        let (_, m) = score(&query_chars("一時"), &Text::new("更新を一時停止する")).unwrap();
        assert_eq!(m, vec![3, 4]);
    }

    #[test]
    fn word_heads_beat_plain_scatter() {
        // W・U がどちらも語の頭の物は、片方だけの物より上
        assert!(s("wu", "WindowsUpdate").unwrap() > s("wu", "awkward union").unwrap());
    }

    #[test]
    fn step_wraps_one_at_a_time_and_clamps_pages() {
        assert_eq!(step(0, -1, 5), 4);
        assert_eq!(step(4, 1, 5), 0);
        assert_eq!(step(1, -PAGE, 5), 0);
        assert_eq!(step(1, PAGE, 5), 4);
        assert_eq!(step(3, 1, 0), 0);
    }

    #[test]
    fn empty_query_lists_tabs_and_ops_only_and_typed_word_ranks_tab_first() {
        let st = crate::demo::state();
        let fixed = fixed_items(&st);
        let pool = pool_items(&st);
        assert!(pool.iter().any(|i| i.kind == Kind::Proc) && pool.iter().any(|i| i.kind == Kind::Service));
        let (hits, total) = rank(&[], &fixed, &pool);
        assert_eq!(total, fixed.len());
        assert!(hits.iter().all(|h| matches!(h.item.kind, Kind::Tab | Kind::Op)));
        let (hits, _) = rank(&query_chars("サービス"), &fixed, &pool);
        assert_eq!(hits[0].item.cmd, Cmd::Tab(Tab::Services));
        let (hits, _) = rank(&query_chars("note"), &fixed, &pool);
        assert!(matches!(hits[0].item.cmd, Cmd::Proc(_)), "{:?}", hits[0].item.title.chars);
        // PID でも探せる
        let pid = st.snap.rows.iter().find(|r| r.name == "notepad.exe").unwrap().id.pid;
        let (hits, _) = rank(&query_chars(&pid.to_string()), &fixed, &pool);
        assert!(hits.iter().any(|h| h.item.sub.chars.iter().collect::<String>() == format!("PID {pid}")));
    }

    #[test]
    fn restart_elevated_only_when_not_elevated() {
        let mut st = crate::demo::state();
        st.env.elevated = false;
        assert!(fixed_items(&st).iter().any(|i| i.cmd == Cmd::RestartElevated));
        st.env.elevated = true;
        assert!(!fixed_items(&st).iter().any(|i| i.cmd == Cmd::RestartElevated));
    }
}
