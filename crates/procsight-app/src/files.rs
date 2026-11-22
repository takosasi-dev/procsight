//! 設定(config.toml。§9.3・D-30・E-41)と一時停止の記録(suspended.toml。§9.9・D-37)のファイルの読み書きと、時刻の文字。
//! 書くときは一時ファイルに書いてから置き換える。suspended.toml は std だけで読み書きする(形が小さいので)。
//! ここで扱う値にフルパス・ユーザー名・コマンドラインは入れない(INV-7)。previous_debugger は §9.3 の決まりで設定に入る。

use procsight_core::ProcId;
use procsight_core::config::{Config, LoadError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 1601 年から 1970 年までの 100 ナノ秒
const EPOCH_100NS: u64 = 116_444_736_000_000_000;

pub fn now_filetime() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    EPOCH_100NS + d.as_nanos() as u64 / 100
}

/// `2026-11-21T23:59:59.123+09:00`(§9.8 のログの形)
pub fn iso_time(ft: u64, offset_minutes: i32, with_ms: bool) -> String {
    let base = procsight_core::format::filetime(ft, offset_minutes).replacen(' ', "T", 1);
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let off = offset_minutes.unsigned_abs();
    let ms = if with_ms { format!(".{:03}", ft / 10_000 % 1000) } else { String::new() };
    format!("{base}{ms}{sign}{:02}:{:02}", off / 60, off % 60)
}

/// `YYYYMMDD-HHMMSS`(壊れた設定の退避の名前)
pub fn stamp(ft: u64, offset_minutes: i32) -> String {
    let s = procsight_core::format::filetime(ft, offset_minutes);
    s.replace(['-', ':'], "").replace(' ', "-")
}

/// `%APPDATA%\ProcSight`
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("ProcSight"))
}

/// `%LOCALAPPDATA%\ProcSight`
pub fn local_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("ProcSight"))
}

/// 読み込みの結果。`broken` は退避したファイルの名前(帯に出す。E-41)
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
    pub broken: Option<String>,
}

pub fn load_config(dir: &Path, offset_minutes: i32) -> Loaded {
    let path = dir.join("config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { config: Config::default(), warnings: Vec::new(), broken: None };
        }
        // UTF-8 でない・読めないも壊れた扱い
        Err(e) => return broken(&path, offset_minutes, e.to_string()),
    };
    match Config::parse(&text) {
        Ok((config, warnings)) => Loaded { config, warnings, broken: None },
        Err(LoadError::Broken(m)) => broken(&path, offset_minutes, m),
    }
}

fn broken(path: &Path, offset_minutes: i32, _reason: String) -> Loaded {
    let name = format!("config.toml.broken-{}", stamp(now_filetime(), offset_minutes));
    let moved = std::fs::rename(path, path.with_file_name(&name)).is_ok();
    Loaded {
        config: Config::default(),
        // 理由の文には設定の値(パスなど)が入りうるので、ログに流れる warnings には書かない(INV-7)
        warnings: vec!["設定を読めませんでした(形が正しくありません)".into()],
        broken: moved.then_some(name),
    }
}

/// 一時ファイルに書いてから置き換える
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

pub fn save_config(dir: &Path, c: &Config) -> std::io::Result<()> {
    write_atomic(&dir.join("config.toml"), &c.to_toml())
}

/// suspended.toml の 1 件
#[derive(Clone, Debug, PartialEq)]
pub struct SuspendedEntry {
    pub id: ProcId,
    pub name: String,
    pub suspended_at: String,
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for c in inner.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

pub fn suspended_to_text(list: &[SuspendedEntry]) -> String {
    let mut out = String::new();
    for e in list {
        out.push_str("[[suspended]]\n");
        out.push_str(&format!("pid = {}\n", e.id.pid));
        out.push_str(&format!("create_time = {}\n", e.id.create_time));
        out.push_str(&format!("name = {}\n", quote(&e.name)));
        out.push_str(&format!("suspended_at = {}\n\n", quote(&e.suspended_at)));
    }
    out
}

/// 読めない行は飛ばす。pid と create_time がそろった物だけ返す
pub fn suspended_from_text(text: &str) -> Vec<SuspendedEntry> {
    let mut out = Vec::new();
    let mut cur: Option<(Option<u32>, Option<u64>, String, String)> = None;
    let mut flush = |cur: &mut Option<(Option<u32>, Option<u64>, String, String)>| {
        if let Some((Some(pid), Some(ct), name, at)) = cur.take() {
            out.push(SuspendedEntry { id: ProcId { pid, create_time: ct }, name, suspended_at: at });
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line == "[[suspended]]" {
            flush(&mut cur);
            cur = Some((None, None, String::new(), String::new()));
            continue;
        }
        let (Some(c), Some((k, v))) = (cur.as_mut(), line.split_once('=')) else { continue };
        match k.trim() {
            "pid" => c.0 = v.trim().parse().ok(),
            "create_time" => c.1 = v.trim().parse().ok(),
            "name" => c.2 = unquote(v).unwrap_or_default(),
            "suspended_at" => c.3 = unquote(v).unwrap_or_default(),
            _ => {}
        }
    }
    flush(&mut cur);
    out
}

pub fn load_suspended(dir: &Path) -> Vec<SuspendedEntry> {
    std::fs::read_to_string(dir.join("suspended.toml")).map(|t| suspended_from_text(&t)).unwrap_or_default()
}

/// 空なら消す
pub fn save_suspended(dir: &Path, list: &[SuspendedEntry]) -> std::io::Result<()> {
    let path = dir.join("suspended.toml");
    if list.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    write_atomic(&path, &suspended_to_text(list))
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("procsight-app-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        let ft = (1_795_305_599u64 + 11_644_473_600) * 10_000_000 + 123 * 10_000;
        assert_eq!(iso_time(ft, 9 * 60, true), "2026-11-22T08:59:59.123+09:00");
        assert_eq!(iso_time(ft, -90, false), "2026-11-21T22:29:59-01:30");
        assert_eq!(stamp(ft, 9 * 60), "20261122-085959");
    }

    #[test]
    fn suspended_round_trip() {
        let list = vec![
            SuspendedEntry {
                id: ProcId { pid: 1234, create_time: 133_000_000_000_000_000 },
                name: "foo \"x\" \\.exe".into(),
                suspended_at: "2026-11-21T23:59:59+09:00".into(),
            },
            SuspendedEntry { id: ProcId { pid: 7, create_time: 1 }, name: "b.exe".into(), suspended_at: String::new() },
        ];
        let text = suspended_to_text(&list);
        assert!(text.starts_with("[[suspended]]\npid = 1234\ncreate_time = 133000000000000000\n"));
        assert_eq!(suspended_from_text(&text), list);
        assert_eq!(suspended_from_text("pid = 3\n[[suspended]]\npid = x\n"), vec![], "壊れた物は捨てる");
    }

    #[test]
    fn suspended_file_is_removed_when_empty() {
        let d = temp_dir("susp");
        let e = SuspendedEntry { id: ProcId { pid: 1, create_time: 2 }, name: "a.exe".into(), suspended_at: String::new() };
        save_suspended(&d, std::slice::from_ref(&e)).unwrap();
        assert_eq!(load_suspended(&d), vec![e]);
        save_suspended(&d, &[]).unwrap();
        assert!(!d.join("suspended.toml").exists());
        save_suspended(&d, &[]).unwrap();
    }

    #[test]
    fn broken_config_is_moved_aside() {
        // E-41: 壊れたファイルを退避して既定で起動する
        let d = temp_dir("cfg");
        std::fs::write(d.join("config.toml"), "interval_ms = [1").unwrap();
        let l = load_config(&d, 540);
        assert_eq!(l.config, Config::default());
        let name = l.broken.expect("退避した名前");
        assert!(name.starts_with("config.toml.broken-"), "{name}");
        assert!(d.join(&name).exists() && !d.join("config.toml").exists());
        // 書いて読み直すと同じ。AC-48 の範囲外の値は戻る
        let c = Config { interval_ms: 2000, ..Config::default() };
        save_config(&d, &c).unwrap();
        assert_eq!(load_config(&d, 0).config.interval_ms, 2000);
        std::fs::write(d.join("config.toml"), "interval_ms = 123\n").unwrap();
        let l = load_config(&d, 0);
        assert_eq!((l.config.interval_ms, l.warnings.len()), (1000, 1));
        assert!(load_config(&d.join("nothing"), 0).broken.is_none());
    }
}
