//! プロセスの一覧の CSV での保存(v0.3.0)。今の表に見えている行 × 見えている列を、表計算で開ける形にする。
//! 保存先は Job::PickSaveFile で選び、書くのは std::fs だけ。選んだパスはログに書かない(INV-7)。
//! 名前やコマンドラインは誰でも付けられるので、式に見える欄は前に ' を付ける(表計算での式の注入を防ぐ)。

use crate::app::{BandKind, State};
use crate::{log, procs};
use procsight_core::format;
use procsight_win::jobs::Job;

/// メニュー・パレットの「一覧を CSV で保存…」。保存先を選ぶ Job を積む(最初のファイル名はローカルの今の時刻)
pub fn request(st: &mut State) {
    // 画面のスレッドから procsight-win を呼ばない(INV-9)。今の時刻は std から FILETIME にする
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let suggested = suggested_name((unix + 11_644_473_600) * 10_000_000, st.env.utc_offset_minutes);
    st.job_picker(Job::PickSaveFile { suggested });
}

/// `procsight-YYYYMMDD-HHMMSS.csv`(FILETIME と UTC からのずれ(分)から)
pub fn suggested_name(now_ft: u64, offset_minutes: i32) -> String {
    let digits: String = format::filetime(now_ft, offset_minutes).chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 14 { format!("procsight-{}-{}.csv", &digits[..8], &digits[8..]) } else { "procsight.csv".into() }
}

/// 保存先が決まった(やめたら None)。書けたら成功の帯、書けなければ失敗の帯
pub fn on_picked(st: &mut State, path: Option<String>) {
    let Some(path) = path else { return };
    // 表を描いていない間(別のタブ・ミニ表示)でも、今の Snapshot・絞り込み・畳みで並びを作り直してから書く
    procs::refresh(st);
    let (text, rows) = csv_text(st);
    match std::fs::write(&path, text.as_bytes()) {
        Ok(()) => {
            log::info("export_csv", &format!("rows={rows}"));
            st.ok(format!("一覧を CSV で保存しました({rows} 行)"));
        }
        Err(e) => {
            log::warn("export_csv_failed", &format!("kind={:?} code={}", e.kind(), e.raw_os_error().unwrap_or(0)));
            st.band(BandKind::Danger, format!("CSV を保存できませんでした。{}", io_reason(&e)));
        }
    }
}

/// 書けなかった理由の短い文(OpError の文はプロセスの操作向けなので使わない)
fn io_reason(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(32 | 33) => "ほかのアプリがファイルを開いています。表計算ソフトなどで開いていれば閉じてから、もう一度保存してください".into(),
        Some(5) => "この場所に書き込む権限がありません。別の場所を選んでください".into(),
        Some(3) => "保存先のフォルダが見つかりません".into(),
        Some(39 | 112) => "ディスクの空きが足りません".into(),
        // std の文は Windows のエラーの文(FormatMessage)を含む。画面のスレッドから procsight-win を呼ばない(INV-9)
        Some(_) => e.to_string(),
        None => e.to_string(),
    }
}

/// 表計算で式に読まれる先頭の文字(全角の形も Excel は式として読むことがある)
const FORMULA_HEADS: [char; 11] = ['=', '+', '-', '@', '\t', '\r', '\n', '\u{FF1D}', '\u{FF0B}', '\u{FF0D}', '\u{FF20}'];

/// 式に見える欄は前に ' を付ける。先頭の空白(改行・NBSP・全角の空白を含む全部)の後ろも見る(空白を落として読む表計算がある)。
/// 先頭そのものが制御文字(タブ・改行)なら、空白を落とす前にそれだけで付ける
fn guard(s: String) -> String {
    let first = s.chars().next();
    let head = s.trim_start_matches(char::is_whitespace).chars().next();
    if first.is_some_and(|c| FORMULA_HEADS.contains(&c)) || head.is_some_and(|c| FORMULA_HEADS.contains(&c)) {
        format!("'{s}")
    } else {
        s
    }
}

/// 1 行を書く。全部の欄を " で囲み、中の " は二重にする。行末は CRLF
fn push_line(out: &mut String, fields: impl Iterator<Item = String>) {
    for (i, f) in fields.enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&guard(f).replace('"', "\"\""));
        out.push('"');
    }
    out.push_str("\r\n");
}

/// CSV の中身と、書いた行の数(見出しを除く)。今の表に見えている行(絞り込み・範囲・ツリーの畳みの後。
/// 絞り込みで弱い文字で残した親も含む)× 見えている列。1 行目は列の題。UTF-8 の BOM つき。
/// 値は表と同じ文字(コピーと同じ)。CPU の推移の列は今の %。ツリーでも名前に字下げは入れない(並びはツリーの順)。
/// 並びは `procs::refresh` 済みであること(on_picked が呼ぶ)
pub fn csv_text(st: &State) -> (String, usize) {
    let cols = st.cfg.columns.visible_columns();
    let mut out = String::with_capacity(64 + st.procs.visible.len() * cols.len() * 16);
    out.push('\u{FEFF}');
    push_line(&mut out, cols.iter().map(|c| procs::column_title(st, *c).to_string()));
    let mut rows = 0;
    for v in &st.procs.visible {
        let Some(r) = st.snap.rows.get(v.index) else { continue };
        push_line(&mut out, cols.iter().map(|c| procs::cell_text(st, r, *c)));
        rows += 1;
    }
    (out, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procsight_core::Column;
    use procsight_core::config::View;
    use std::sync::Arc;

    /// 見本の状態で、名前を変えた notepad.exe だけが見える形
    fn one_row(name: &str) -> State {
        let mut st = crate::demo::state();
        let snap = Arc::make_mut(&mut st.snap);
        snap.rows.iter_mut().find(|r| r.name == "notepad.exe").unwrap().name = name.into();
        st.cfg.view = View::Flat;
        st.cfg.columns.set_visible_columns(&[Column::Name, Column::Pid, Column::CpuHistory]);
        st.procs.filter = "pid:7000".into();
        st.procs.invalidate();
        procs::refresh(&mut st);
        st
    }

    fn lines(text: &str) -> Vec<&str> {
        text.strip_prefix('\u{FEFF}').expect("BOM で始まる").strip_suffix("\r\n").expect("CRLF で終わる").split("\r\n").collect()
    }

    #[test]
    fn bom_crlf_header_and_visible_columns_only() {
        let st = one_row("notepad.exe");
        let (text, n) = csv_text(&st);
        assert_eq!(n, 1);
        assert!(text.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]), "UTF-8 の BOM");
        assert!(!text.replace("\r\n", "").contains('\n'), "行末は全部 CRLF");
        let l = lines(&text);
        assert_eq!(l[0], "\"名前\",\"PID\",\"CPU の推移\"");
        let r = st.snap.rows.iter().find(|r| r.id.pid == 7000).unwrap();
        assert_eq!(l[1], format!("\"notepad.exe\",\"7000\",\"{}\"", format::pct(r.cpu_pct)));
        assert!(l[1].ends_with("%\""), "CPU の推移は今の %");
    }

    #[test]
    fn formula_injection_is_neutralised() {
        for (name, want) in [
            ("=HYPERLINK(\"x\")", "\"'=HYPERLINK(\"\"x\"\")\""),
            ("+1", "\"'+1\""),
            ("-2+3", "\"'-2+3\""),
            ("@SUM(A1)", "\"'@SUM(A1)\""),
            ("\tcmd", "\"'\tcmd\""),
            ("\rcmd", "\"'\rcmd\""),
            ("  =1+1", "\"'  =1+1\""),
            ("\u{FF1D}1+1", "\"'\u{FF1D}1+1\""),
            ("\u{00A0}=1+1", "\"'\u{00A0}=1+1\""),
            ("\n=1+1", "\"'\n=1+1\""),
            ("a=b.exe", "\"a=b.exe\""),
            // 値が無い欄の「—」(U+2014)は式ではない
            (format::DASH, "\"\u{2014}\""),
        ] {
            let (text, _) = csv_text(&one_row(name));
            let row = text.split("\r\n").nth(1).unwrap();
            assert!(row.starts_with(&format!("{want},")), "{name:?} → {row:?}");
        }
    }

    #[test]
    fn quotes_and_newlines_stay_inside_one_field() {
        let (text, n) = csv_text(&one_row("a \"quoted\"\r\nnext,line.exe"));
        assert_eq!(n, 1);
        assert!(text.contains("\"a \"\"quoted\"\"\r\nnext,line.exe\",\"7000\","), "{text:?}");
    }

    #[test]
    fn follows_filter_tree_and_collapse() {
        let mut st = crate::demo::state();
        st.cfg.view = View::Tree;
        st.procs.filter = "testee".into();
        st.procs.invalidate();
        procs::refresh(&mut st);
        let (text, n) = csv_text(&st);
        assert_eq!(n, st.procs.visible.len());
        assert!(n > 13, "一致した 13 行と、弱い文字で残した親");
        assert_eq!(lines(&text).len(), n + 1);
        // 根を畳むと子と孫は書かない
        let root = st.snap.rows.iter().find(|r| r.id.pid == 10_000).unwrap().id;
        st.procs.collapsed.insert(root);
        st.procs.invalidate();
        procs::refresh(&mut st);
        let (_, folded) = csv_text(&st);
        assert_eq!(folded, n - 12);
    }

    #[test]
    fn suggested_name_is_local_time() {
        // 2026-11-22 12:34:56 UTC を、日本(+9 時間)で
        let unix = 1_795_350_896u64;
        let ft = (unix + 11_644_473_600) * 10_000_000;
        assert_eq!(suggested_name(ft, 9 * 60), "procsight-20261122-213456.csv");
        assert_eq!(suggested_name(ft, 0), "procsight-20261122-123456.csv");
    }
}
