//! config.toml の型(仕様書 §9.3)と、範囲の外の値を既定に戻す処理(D-30・AC-48)。
//! ファイルの読み書き(一時ファイルからの置き換え・壊れたファイルの退避)は app が行う。
//! 仕様書 §9.3 の例では `fonts` が `[taskmgr_replace]` の下に入ってしまうので、根の項目として扱う。

use crate::model::Column;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const INTERVALS_MS: [u64; 4] = [500, 1000, 2000, 4000];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Dark,
    Light,
    #[default]
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Tree,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    #[default]
    Exit,
    Tray,
}

/// 表の行の高さ(v0.2.0)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Normal, Density::Comfortable];
    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "詰める",
            Density::Normal => "標準",
            Density::Comfortable => "ゆったり",
        }
    }
    /// 表の 1 行の高さ(ポイント)
    pub fn row_h(self) -> f32 {
        match self {
            Density::Compact => 18.0,
            Density::Normal => 22.0,
            Density::Comfortable => 26.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaskmgrReplace {
    pub enabled: bool,
    pub previous_debugger: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontChoice {
    pub path: String,
    #[serde(default)]
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchFree {
    pub names: Vec<String>,
    pub include_descendants: bool,
}

impl Default for TouchFree {
    fn default() -> Self {
        TouchFree { names: Vec::new(), include_descendants: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortSetting {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub visible: Vec<String>,
    pub widths: BTreeMap<String, f32>,
    pub sort: SortSetting,
}

pub const DEFAULT_COLUMNS: [Column; 9] = [
    Column::Name,
    Column::Pid,
    Column::State,
    Column::Cpu,
    Column::Mem,
    Column::IoRead,
    Column::IoWrite,
    Column::Gpu,
    Column::User,
];

impl Default for Columns {
    fn default() -> Self {
        Columns {
            visible: DEFAULT_COLUMNS.iter().map(|c| c.key().to_string()).collect(),
            widths: BTreeMap::new(),
            sort: SortSetting { column: "cpu".into(), descending: true },
        }
    }
}

impl Columns {
    /// 表示する列(名前の列は外せないので必ず含める)
    pub fn visible_columns(&self) -> Vec<Column> {
        let mut out: Vec<Column> = self.visible.iter().filter_map(|k| Column::from_key(k)).collect();
        if !out.contains(&Column::Name) {
            out.insert(0, Column::Name);
        }
        out
    }
    pub fn set_visible_columns(&mut self, cols: &[Column]) {
        self.visible = cols.iter().map(|c| c.key().to_string()).collect();
    }
    pub fn sort_column(&self) -> Column {
        Column::from_key(&self.sort.column).unwrap_or(Column::Cpu)
    }
    pub fn width(&self, c: Column) -> f32 {
        self.widths.get(c.key()).copied().unwrap_or_else(|| c.default_width())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowPlacement {
    fn default() -> Self {
        WindowPlacement { x: 100, y: 100, width: 1100, height: 720, maximized: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub interval_ms: u64,
    pub theme: Theme,
    pub font_scale: f32,
    pub view: View,
    pub close_action: CloseAction,
    pub tray_enabled: bool,
    pub elevate_on_start: bool,
    pub fonts: Vec<FontChoice>,
    pub taskmgr_replace: TaskmgrReplace,
    pub touch_free: TouchFree,
    pub columns: Columns,
    pub window: WindowPlacement,
    /// 数字の列の背景を使用量で塗る(v0.2.0)
    pub heat_map: bool,
    /// 表の行の高さ(v0.2.0)
    pub density: Density,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema_version: 1,
            interval_ms: 1000,
            theme: Theme::System,
            font_scale: 1.0,
            view: View::Tree,
            close_action: CloseAction::Exit,
            tray_enabled: false,
            elevate_on_start: false,
            fonts: Vec::new(),
            taskmgr_replace: TaskmgrReplace::default(),
            touch_free: TouchFree::default(),
            columns: Columns::default(),
            window: WindowPlacement::default(),
            heat_map: true,
            density: Density::Normal,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// TOML として読めない。app はファイルを `config.toml.broken-…` に退避して既定で起動する(E-41)
    Broken(String),
}

impl Config {
    /// 読み込み。範囲の外の値は既定に戻し、戻した項目ごとに警告を 1 つ返す
    pub fn parse(text: &str) -> Result<(Config, Vec<String>), LoadError> {
        let mut c: Config = toml::from_str(text).map_err(|e| LoadError::Broken(e.message().to_string()))?;
        let warnings = c.sanitize();
        Ok((c, warnings))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("Config は常に TOML にできる")
    }

    /// 範囲の外の値を既定に戻す
    pub fn sanitize(&mut self) -> Vec<String> {
        let d = Config::default();
        let mut w = Vec::new();
        if self.schema_version != 1 {
            w.push(format!("schema_version={} を 1 に戻しました", self.schema_version));
            self.schema_version = 1;
        }
        if !INTERVALS_MS.contains(&self.interval_ms) {
            w.push(format!("interval_ms={} を {} に戻しました", self.interval_ms, d.interval_ms));
            self.interval_ms = d.interval_ms;
        }
        if !(0.8..=1.6).contains(&self.font_scale) || !self.font_scale.is_finite() {
            w.push(format!("font_scale={} を 1.0 に戻しました", self.font_scale));
            self.font_scale = 1.0;
        }
        if self.close_action == CloseAction::Tray && !self.tray_enabled {
            // tray_enabled が false なら exit とみなす(§9.3)。値は残す
        }
        let names = crate::rules::normalize_touch_free_list(self.touch_free.names.iter().map(|s| s.as_str()));
        if names != self.touch_free.names {
            w.push("touch_free.names を直しました(小文字・重複・空・100 個まで)".into());
            self.touch_free.names = names;
        }
        let before = self.columns.visible.len();
        self.columns.visible.retain(|k| Column::from_key(k).is_some());
        let mut seen = std::collections::HashSet::new();
        self.columns.visible.retain(|k| seen.insert(k.clone()));
        if self.columns.visible.len() != before {
            w.push("columns.visible の知らない列・重複を外しました".into());
        }
        if self.columns.visible.is_empty() {
            self.columns.visible = d.columns.visible.clone();
        }
        let before = self.columns.widths.len();
        self.columns.widths.retain(|k, v| Column::from_key(k).is_some() && (40.0..=800.0).contains(v));
        if self.columns.widths.len() != before {
            w.push("columns.widths の範囲の外の幅を捨てました".into());
        }
        if Column::from_key(&self.columns.sort.column).is_none() {
            w.push(format!("columns.sort.column={} を cpu に戻しました", self.columns.sort.column));
            self.columns.sort = d.columns.sort.clone();
        }
        let win = &self.window;
        if win.width < 400 || win.height < 300 || win.width > 16_384 || win.height > 16_384 {
            w.push("window の大きさを既定に戻しました".into());
            self.window = WindowPlacement::default();
        }
        w
    }

    pub fn effective_close_action(&self) -> CloseAction {
        if self.tray_enabled { self.close_action } else { CloseAction::Exit }
    }

    pub fn touch_free_set(&self) -> crate::rules::TouchFreeSet {
        crate::rules::TouchFreeSet {
            names: self.touch_free.names.clone(),
            include_descendants: self.touch_free.include_descendants,
            keep: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.1.0 が書いた config.toml(heat_map・density が無い)をそのまま読めて、新しい項目は既定になる
    #[test]
    fn v0_1_0_config_still_loads() {
        let v010 = r#"schema_version = 1
interval_ms = 2000
theme = "dark"
font_scale = 1.2
view = "flat"
close_action = "exit"
tray_enabled = false
elevate_on_start = false
fonts = []

[taskmgr_replace]
enabled = false
previous_debugger = ""

[touch_free]
names = ["acegame.exe"]
include_descendants = true

[columns]
visible = ["name", "pid", "state", "cpu", "mem"]

[columns.sort]
column = "cpu"
descending = true

[window]
x = 100
y = 100
width = 1100
height = 720
maximized = false
"#;
        let (c, w) = Config::parse(v010).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.interval_ms, 2000);
        assert_eq!(c.touch_free.names, vec!["acegame.exe".to_string()]);
        assert!(c.heat_map);
        assert_eq!(c.density, Density::Normal);
        // 書き戻して読み直しても同じ
        let again = Config::parse(&toml::to_string(&c).unwrap()).unwrap().0;
        assert_eq!(again, c);
    }

    #[test]
    fn empty_file_is_default() {
        let (c, w) = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
    }

    #[test]
    fn out_of_range_interval_goes_back_with_one_warning() {
        // AC-48 の読み込みの部分
        let (c, w) = Config::parse("interval_ms = 123\n").unwrap();
        assert_eq!(c.interval_ms, 1000);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn other_ranges() {
        let text = r#"
font_scale = 3.0
[touch_free]
names = ["C:\\Games\\ACE8.exe", "ace8.exe", ""]
[columns]
visible = ["pid", "nope", "pid"]
widths = { name = 240.0, pid = 5.0 }
sort = { column = "zzz" }
[window]
width = 10
"#;
        let (c, w) = Config::parse(text).unwrap();
        assert_eq!(c.font_scale, 1.0);
        assert_eq!(c.touch_free.names, vec!["ace8.exe"]);
        assert_eq!(c.columns.visible, vec!["pid"]);
        assert_eq!(c.columns.visible_columns(), vec![Column::Name, Column::Pid], "名前の列は外せない");
        assert_eq!(c.columns.widths.len(), 1);
        assert_eq!(c.columns.sort_column(), Column::Cpu);
        assert_eq!(c.window, WindowPlacement::default());
        assert_eq!(w.len(), 6);
    }

    #[test]
    fn broken_file() {
        assert!(matches!(Config::parse("interval_ms = [1"), Err(LoadError::Broken(_))));
        assert!(matches!(Config::parse("theme = \"pink\""), Err(LoadError::Broken(_))));
    }

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.touch_free.names = vec!["ace8.exe".into()];
        c.fonts = vec![FontChoice { path: "C:\\Windows\\Fonts\\YuGothR.ttc".into(), index: 0 }];
        c.columns.widths.insert("name".into(), 300.0);
        c.taskmgr_replace.previous_debugger = "\"C:\\x\\procexp.exe\"".into();
        let (back, w) = Config::parse(&c.to_toml()).unwrap();
        assert_eq!(back, c);
        assert!(w.is_empty());
    }

    #[test]
    fn tray_off_means_exit() {
        let c = Config { close_action: CloseAction::Tray, ..Config::default() };
        assert_eq!(c.effective_close_action(), CloseAction::Exit);
    }
}
