//! S3 スタートアップ(画面設計書 §6.1・FR-38〜FR-40)。場所での絞り込み・一覧の表と、有効・無効・場所を開く・取り直す。
//! 全ユーザーの項目は昇格していなければ灰色(FR-39)。状態が「不明」・ストア アプリ(読むだけ)の項目も切り替えない(E-30)。
//! 状態は色の札(有効・無効・不明)、exe が無ければ危険の色の「ファイルなし」。上に件数の要約。

use eframe::egui;
use crate::app::{Load, State};
use crate::services_view::{button, loading, segmented};
use crate::theme::{self, Palette, space};
use crate::widgets;
use egui::{Align, Color32, Layout, RichText, Sense, Ui};
use egui_extras::TableBuilder;
use procsight_core::format;
use procsight_core::startup::{Approved, StartupItem, StartupLocation};
use procsight_win::jobs::{Job, OpError};

/// 場所での絞り込み(v0.3.0。タスクとストア アプリが増えて長くなったため)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LocFilter {
    #[default]
    All,
    Registry,
    Folder,
    Task,
    Store,
}

impl LocFilter {
    pub const ALL: [LocFilter; 5] = [LocFilter::All, LocFilter::Registry, LocFilter::Folder, LocFilter::Task, LocFilter::Store];

    pub fn label(self) -> &'static str {
        match self {
            LocFilter::All => "すべて",
            LocFilter::Registry => "レジストリ",
            LocFilter::Folder => "フォルダ",
            LocFilter::Task => "タスク",
            LocFilter::Store => "ストア アプリ",
        }
    }

    pub fn matches(self, loc: StartupLocation) -> bool {
        use StartupLocation as L;
        match self {
            LocFilter::All => true,
            LocFilter::Registry => matches!(loc, L::RegistryUser | L::RegistryMachine | L::Registry32),
            LocFilter::Folder => matches!(loc, L::FolderUser | L::FolderCommon),
            LocFilter::Task => loc == L::ScheduledTask,
            LocFilter::Store => loc == L::StoreApp,
        }
    }
}

#[derive(Default)]
pub struct StartupView {
    pub items: Load<Vec<StartupItem>>,
    pub selected: Option<(String, StartupLocation)>,
    pub loc_filter: LocFilter,
}

pub fn on_loaded(st: &mut State, r: Result<Vec<StartupItem>, OpError>) {
    st.startup.items = match r {
        Ok(v) => Load::Ready(v),
        Err(e) => Load::Failed(e),
    };
}

pub fn on_set(st: &mut State, name: String, enabled: bool, r: Result<(), OpError>) {
    match r {
        Ok(()) => {
            st.ok(format!("『{name}』を{}しました", if enabled { "有効に" } else { "無効に" }));
            crate::log::info("startup_set", &format!("enabled={enabled} result=ok"));
            st.job_load_startup();
        }
        Err(e) => st.fail(&e),
    }
}

/// 切り替えの灰色の理由(読むだけの項目は、その理由を先に出す)
fn refusal(st: &State, it: &StartupItem) -> Option<&'static str> {
    if let Some(r) = it.readonly_reason {
        Some(r)
    } else if it.location.needs_admin() && !st.env.elevated {
        Some("管理者で再起動すると変えられます")
    } else if it.approved == Approved::Unknown {
        Some("記録の形が想定と違うため、切り替えません")
    } else {
        None
    }
}

/// 状態の色(有効は成功の色、無効は弱い色、不明は警告の色)
pub fn approved_color(pal: &Palette, a: Approved) -> Color32 {
    match a {
        Approved::Enabled => pal.ok,
        Approved::Disabled(_) => pal.weak,
        Approved::Unknown => pal.warn,
    }
}

/// 折り返しの並び(horizontal_wrapped)の終わりに置く、色の点の要約。全部が今の行に入れば区切りの線の後に、
/// 入らなければ次の行の頭から並べる(dot_label は ui.horizontal なので、そのままでは折り返されずに右で切れる)
fn dot_summary(ui: &mut Ui, parts: &[(String, Color32)]) {
    let gap = ui.spacing().item_spacing.x;
    let width = |ui: &Ui, t: &str| {
        let g = egui::WidgetText::from(t).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
        g.size().x + 8.0 + space::S + 1.0 + gap
    };
    // 折り返しの並びの available_width は行の全幅を返すので、カーソルから右の端までで測る
    let left = |ui: &Ui| ui.max_rect().right() - ui.cursor().left();
    let total: f32 = parts.iter().map(|(t, _)| width(ui, t)).sum::<f32>() + gap * 2.0 + 1.0;
    if left(ui) < total {
        ui.end_row();
    } else {
        ui.separator();
    }
    for (t, c) in parts {
        if left(ui) < width(ui, t) {
            ui.end_row();
        }
        widgets::dot_label(ui, t, *c);
    }
}

pub fn ui(st: &mut State, ui: &mut Ui) {
    if st.startup.items.is_not_asked() {
        st.job_load_startup();
    }
    let pal = theme::palette(ui);
    let filter = st.startup.loc_filter;
    // 絞り込みで見えている行(並びは一覧のまま)。選択は見えている行からだけ取る(隠れた行にボタンが効かない)
    let visible: Vec<StartupItem> = match &st.startup.items {
        Load::Ready(v) => v.iter().filter(|i| filter.matches(i.location)).cloned().collect(),
        _ => Vec::new(),
    };
    let sel = st.startup.selected.as_ref().and_then(|(n, l)| visible.iter().find(|i| &i.name == n && i.location == *l).cloned());
    ui.horizontal_wrapped(|ui| {
        if let Load::Ready(v) = &st.startup.items
            && !v.is_empty()
        {
            let items: Vec<(LocFilter, String)> = LocFilter::ALL
                .iter()
                .map(|&f| (f, format!("{} {}", f.label(), v.iter().filter(|i| f.matches(i.location)).count())))
                .collect();
            segmented(ui, &pal, &mut st.startup.loc_filter, &items);
            ui.add_space(space::M);
        }
        let none = sel.is_none().then_some("項目を選んでください");
        let why = none.or_else(|| sel.as_ref().and_then(|i| refusal(st, i)));
        if button(ui, "有効にする", why.or(sel.as_ref().filter(|i| i.approved == Approved::Enabled).map(|_| "すでに有効です"))) {
            let it = sel.clone().unwrap();
            st.job(Job::SetStartup { name: it.name, location: it.location, enabled: true });
        }
        let already_off = sel.as_ref().filter(|i| matches!(i.approved, Approved::Disabled(_))).map(|_| "すでに無効です");
        if button(ui, "無効にする", why.or(already_off)) {
            let it = sel.clone().unwrap();
            st.job(Job::SetStartup { name: it.name, location: it.location, enabled: false });
        }
        // タスクの切り替えはタスク全体に効く(ログオン以外のトリガーや 2 つ目以降の動作も止まる)ことを先に知らせる
        if sel.as_ref().is_some_and(|i| i.location == StartupLocation::ScheduledTask) {
            ui.label(RichText::new("タスク全体を切り替えます").color(pal.weak).size(theme::size::SMALL))
                .on_hover_text("タスク スケジューラのタスクは、ログオン時以外のトリガーや 2 つ目以降の動作も含めて、まとめて有効・無効になります");
        }
        let no_exe = none.or(sel.as_ref().and_then(|i| match (&i.exe_path, i.location) {
            (None, StartupLocation::StoreApp) => Some("ストア アプリには開くファイルがありません"),
            (None, _) => Some("ファイルが見つかりません"),
            (Some(_), _) if !i.exe_exists => Some("ファイルが見つかりません"),
            _ => None,
        }));
        if button(ui, "ファイルの場所を開く", no_exe) {
            st.job(Job::OpenLocation { path: sel.as_ref().and_then(|i| i.exe_path.clone()).unwrap_or_default() });
        }
        ui.separator();
        if ui.button("取り直す").on_hover_text("F5").clicked() {
            st.job_load_startup();
        }
        // 件数の要約(見えている行の数。有効・無効は常に、不明とファイルなしはあるときだけ)
        if !visible.is_empty() {
            let on = visible.iter().filter(|i| i.approved == Approved::Enabled).count();
            let off = visible.iter().filter(|i| matches!(i.approved, Approved::Disabled(_))).count();
            let unknown = visible.len() - on - off;
            let missing = visible.iter().filter(|i| !i.exe_exists).count();
            let mut parts = vec![(format!("有効 {on}"), pal.ok), (format!("無効 {off}"), pal.weak)];
            if unknown > 0 {
                parts.push((format!("不明 {unknown}"), pal.warn));
            }
            if missing > 0 {
                parts.push((format!("ファイルなし {missing}"), pal.danger));
            }
            dot_summary(ui, &parts);
        }
    });
    ui.add_space(space::XS);
    ui.separator();
    match &st.startup.items {
        Load::Ready(v) if v.is_empty() => {
            widgets::empty_state(ui, &pal, "ログオン時に動く項目はありません", None);
            return;
        }
        Load::Ready(_) => {}
        Load::Failed(e) => {
            let text = format!("スタートアップの一覧を読めませんでした({})", e.message(st.env.elevated));
            widgets::empty_state(ui, &pal, &text, None);
            return;
        }
        _ => {
            loading(ui, &pal, "読み込んでいます");
            return;
        }
    }
    if visible.is_empty() {
        widgets::empty_state(ui, &pal, "この場所の項目はありません", None);
        return;
    }
    let items = visible;
    let row_h = st.cfg.density.row_h();
    // 最初の幅は窓の幅の割合(狭い窓でも最後の列まで入る)。後は利用者が動かした幅を egui が覚える
    let w = ui.available_width();
    let col = |frac: f32, min: f32| egui_extras::Column::initial((w * frac).max(min)).at_least(min).clip(true);
    let mut click = None;
    TableBuilder::new(ui)
        .id_salt("startup")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(col(0.15, 70.0))
        .column(col(0.15, 70.0))
        .column(col(0.12, 120.0))
        .column(col(0.18, 80.0))
        .column(col(0.12, 60.0))
        .column(egui_extras::Column::remainder().at_least(60.0).clip(true))
        .header(row_h + 2.0, |mut h| {
            for t in ["名前", "発行元", "状態", "場所", "無効にした日時", "コマンド"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, items.len(), |mut row| {
                let it = &items[row.index()];
                row.set_selected(st.startup.selected.as_ref().is_some_and(|(n, l)| *n == it.name && *l == it.location));
                row.col(|ui| {
                    ui.add(egui::Label::new(&it.name).truncate());
                });
                row.col(|ui| {
                    let (text, color) = match it.publisher.as_deref() {
                        Some(p) => (p, ui.visuals().text_color()),
                        None => (format::DASH, pal.weak),
                    };
                    ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
                });
                row.col(|ui| {
                    ui.spacing_mut().item_spacing.x = space::S;
                    widgets::pill(ui, it.approved.label(), approved_color(&pal, it.approved));
                    if !it.exe_exists {
                        widgets::pill(ui, "ファイルなし", pal.danger);
                    }
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(it.location.label()).truncate());
                });
                row.col(|ui| {
                    let when = match it.approved {
                        Approved::Disabled(ft) if ft != 0 => format::filetime(ft, st.env.utc_offset_minutes)[..16].to_string(),
                        _ => format::DASH.into(),
                    };
                    ui.add(egui::Label::new(RichText::new(when).color(pal.weak)).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(&it.command).color(pal.weak)).truncate());
                });
                let resp = row.response();
                if resp.clicked() {
                    click = Some((it.name.clone(), it.location));
                }
                if let Some(r) = it.readonly_reason {
                    resp.on_hover_text(r);
                }
            });
        });
    if click.is_some() {
        st.startup.selected = click;
    }
}
