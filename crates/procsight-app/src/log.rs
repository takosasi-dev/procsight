//! ログの小さな書き手(§9.8。std だけ)。`%LOCALAPPDATA%\ProcSight\logs\procsight-YYYYMMDD.log`、UTF-8・LF、7 日分を残す。
//! 書いてよいのはプロセス名・PID・数・ファイル名まで。コマンドライン・フルパス・ハンドルの名前・ユーザー名は書かない(INV-7)。
//! init を呼ぶ前(テスト)は何も書かない。

use crate::files::{iso_time, now_filetime};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

struct Logger {
    dir: PathBuf,
    offset_minutes: i32,
    file: Mutex<Option<(String, std::fs::File)>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// 残す日数
const KEEP_DAYS: u64 = 7;

fn day(ft: u64, offset: i32) -> String {
    procsight_core::format::filetime(ft, offset)[..10].replace('-', "")
}

/// 起動時に 1 回。7 日より古いログを消す
pub fn init(dir: PathBuf, offset_minutes: i32) {
    let oldest = day(now_filetime().saturating_sub(KEEP_DAYS * 86_400 * 10_000_000), offset_minutes);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(d) = name.strip_prefix("procsight-").and_then(|n| n.strip_suffix(".log"))
                && d.len() == 8
                && d < oldest.as_str()
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = LOGGER.set(Logger { dir, offset_minutes, file: Mutex::new(None) });
}

fn write(level: &str, event: &str, fields: &str) {
    let Some(l) = LOGGER.get() else { return };
    let ft = now_filetime();
    let line = format!("{} {level:<5} event={event}{}{fields}\n", iso_time(ft, l.offset_minutes, true), if fields.is_empty() { "" } else { " " });
    let today = day(ft, l.offset_minutes);
    let Ok(mut guard) = l.file.lock() else { return };
    if guard.as_ref().is_none_or(|(d, _)| *d != today) {
        let _ = std::fs::create_dir_all(&l.dir);
        let path = l.dir.join(format!("procsight-{today}.log"));
        *guard = std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(|f| (today, f));
    }
    if let Some((_, f)) = guard.as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(event: &str, fields: &str) {
    write("INFO", event, fields);
}

pub fn warn(event: &str, fields: &str) {
    write("WARN", event, fields);
}

/// プロセス名に空白や = が入っても 1 つの値に見えるよう、引用符で囲む(パスは渡さないこと)
pub fn q(name: &str) -> String {
    if name.chars().any(|c| c.is_whitespace() || c == '=' || c == '"') {
        format!("\"{}\"", name.replace('"', "'"))
    } else {
        name.to_string()
    }
}

/// パスのファイル名の部分だけ(INV-7)
pub fn file_name(path: &str) -> &str {
    procsight_core::startup::file_name(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_names() {
        assert_eq!(q("foo.exe"), "foo.exe");
        assert_eq!(q("a b.exe"), "\"a b.exe\"");
        assert_eq!(file_name("C:\\Users\\me\\x.ttc"), "x.ttc");
        assert_eq!(day((1_795_305_599u64 + 11_644_473_600) * 10_000_000, 540), "20261122");
    }
}
