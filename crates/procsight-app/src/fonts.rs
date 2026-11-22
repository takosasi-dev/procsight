//! 日本語のフォントを C:\Windows\Fonts\ から起動時に 1 回読む(D-29・E-36)。既定の英字フォントは無いので、英数字・記号もこれで出す。
//! 候補の順: config の fonts → Yu Gothic UI(YuGothM.ttc。書体は name テーブルの名前で選ぶ)→ メイリオ → MS ゴシック。
//! 読んだファイルと番号は診断に出す。ログにはファイル名だけ書く(INV-7)。

use eframe::egui;
use egui::{FontData, FontDefinitions, FontFamily};
use procsight_core::config::FontChoice;
use std::sync::Arc;

pub use crate::ttc::faces;

// この PC の実物: YuGothR.ttc は「Yu Gothic Regular」と「Yu Gothic UI Semilight」。UI の Regular は YuGothM.ttc の 1 番

/// 読んだフォント(診断の表示用)
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedFont {
    pub path: String,
    pub index: u32,
    pub face: String,
}

/// 既定の候補(ファイル名, 書体の名前)。書体の名前が一致する番号を使う
const DEFAULTS: [(&str, &str); 3] =
    [("YuGothM.ttc", "Yu Gothic UI"), ("meiryo.ttc", "Meiryo"), ("msgothic.ttc", "MS Gothic")];

/// 名前が `want` の書体の番号。「Yu Gothic UI」は「Yu Gothic UI Regular」などにも当たる。太字・細字は避ける
fn pick_face(faces: &[(u32, String)], want: &str) -> Option<u32> {
    let plain = |n: &str| !["bold", "light", "semilight", "semibold", "italic"].iter().any(|w| n.to_lowercase().contains(w));
    faces
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(want) || n.eq_ignore_ascii_case(&format!("{want} Regular")))
        .or_else(|| faces.iter().find(|(_, n)| n.starts_with(want) && plain(n)))
        .map(|(i, _)| *i)
}

fn fonts_dir() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{}\\Fonts", root.trim_end_matches('\\'))
}

/// 候補の順に試して、最初に読めた 1 つを返す
pub fn find(choices: &[FontChoice]) -> Option<(Vec<u8>, LoadedFont)> {
    for c in choices {
        if let Ok(bytes) = std::fs::read(&c.path) {
            let list = faces(&bytes);
            if let Some((_, face)) = list.iter().find(|(i, _)| *i == c.index) {
                let face = face.clone();
                return Some((bytes, LoadedFont { path: c.path.clone(), index: c.index, face }));
            }
        }
    }
    let dir = fonts_dir();
    for (file, want) in DEFAULTS {
        let path = format!("{dir}\\{file}");
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let list = faces(&bytes);
        if let Some(index) = pick_face(&list, want) {
            let face = list.iter().find(|(i, _)| *i == index).map(|(_, n)| n.clone()).unwrap_or_default();
            return Some((bytes, LoadedFont { path, index, face }));
        }
    }
    None
}

/// 読んだフォントだけで FontDefinitions を作る(英字の既定フォントは無い)
pub fn definitions(bytes: Arc<Vec<u8>>, index: u32) -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert("jp".into(), Arc::new(FontData::from_blob(bytes, index)));
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        defs.families.insert(fam, vec!["jp".into()]);
    }
    defs
}

/// 起動時に 1 回。読めなければ eframe の補い(system_font_fallback)だけで出す(E-36)
pub fn install(ctx: &egui::Context, choices: &[FontChoice]) -> Option<LoadedFont> {
    let (bytes, loaded) = find(choices)?;
    ctx.set_fonts(definitions(Arc::new(bytes), loaded.index));
    Some(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_yu_gothic_ui_by_name() {
        let list = vec![(0, "Yu Gothic Regular".to_string()), (1, "Yu Gothic UI Regular".to_string()), (2, "Yu Gothic UI Semilight".to_string())];
        assert_eq!(pick_face(&list, "Yu Gothic UI"), Some(1));
        let m = vec![(0, "Meiryo".to_string()), (1, "Meiryo Italic".to_string()), (2, "Meiryo UI".to_string())];
        assert_eq!(pick_face(&m, "Meiryo"), Some(0));
        assert_eq!(pick_face(&m, "MS Gothic"), None);
    }

    #[test]
    fn broken_bytes_have_no_faces() {
        assert!(faces(b"").is_empty());
        assert!(faces(b"ttcf\0\0\0\0").is_empty());
        assert_eq!(faces(&[0u8; 40]), vec![(0, String::new())]);
    }

    /// この PC の実物で書体の名前を読めるか(ファイルが無い PC では何もしない)
    #[test]
    fn real_yu_gothic_faces() {
        let path = format!("{}\\YuGothM.ttc", fonts_dir());
        let Ok(bytes) = std::fs::read(&path) else { return };
        let list = faces(&bytes);
        assert!(list.len() >= 2, "{list:?}");
        assert_eq!(pick_face(&list, "Yu Gothic UI"), Some(1), "{list:?}");
        // 細字だけの YuGothR.ttc からは UI の書体を選ばない
        if let Ok(r) = std::fs::read(format!("{}\\YuGothR.ttc", fonts_dir())) {
            assert_eq!(pick_face(&faces(&r), "Yu Gothic UI"), None);
        }
    }
}
