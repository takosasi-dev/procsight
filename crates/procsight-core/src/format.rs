//! 画面に出す数字の形(画面設計書 §4.2)。メモリは 1024 進で小数 1 桁、速さは B/s・KB/s・MB/s、値が無ければ「—」。

pub const DASH: &str = "—";

pub fn bytes(v: u64) -> String {
    const K: f64 = 1024.0;
    let f = v as f64;
    if f < K {
        format!("{v} B")
    } else if f < K * K {
        format!("{:.1} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / K / K)
    } else {
        format!("{:.1} GB", f / K / K / K)
    }
}

pub fn opt_bytes(v: Option<u64>) -> String {
    v.map_or_else(|| DASH.into(), bytes)
}

pub fn speed(v: Option<f64>) -> String {
    let Some(v) = v else { return DASH.into() };
    let v = v.max(0.0);
    if v < 1024.0 {
        format!("{v:.0} B/s")
    } else if v < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", v / 1024.0)
    } else if v < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB/s", v / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB/s", v / 1024.0 / 1024.0 / 1024.0)
    }
}

pub fn pct(v: Option<f32>) -> String {
    v.map_or_else(|| DASH.into(), |v| format!("{v:.1}%"))
}

/// 3 桁ごとのカンマ
pub fn thousands(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// CPU 時間(100 ナノ秒単位)を H:MM:SS に
pub fn cpu_time(t100ns: u64) -> String {
    let s = t100ns / 10_000_000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// 稼働時間(ミリ秒)を D:HH:MM:SS に
pub fn uptime(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}:{:02}", s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60)
}

/// FILETIME(1601 年からの 100 ナノ秒)を、UTC からのずれ(分)を足して `YYYY-MM-DD HH:MM:SS` に
pub fn filetime(ft: u64, offset_minutes: i32) -> String {
    if ft == 0 {
        return DASH.into();
    }
    // 1601-01-01 から 1970-01-01 までの秒数
    const EPOCH_DIFF: i64 = 11_644_473_600;
    let secs = (ft / 10_000_000) as i64 - EPOCH_DIFF + offset_minutes as i64 * 60;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

/// 1970-01-01 からの日数を年月日に(Howard Hinnant の days_from_civil の逆)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(112 * 1024 * 1024 + 300 * 1024), "112.3 MB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(opt_bytes(None), "—");
        assert_eq!(speed(Some(0.0)), "0 B/s");
        assert_eq!(speed(Some(12.0 * 1024.0)), "12.0 KB/s");
        assert_eq!(speed(Some(20.0 * 1024.0 * 1024.0)), "20.0 MB/s");
        assert_eq!(pct(Some(18.44)), "18.4%");
        assert_eq!(thousands(151203), "151,203");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn times() {
        assert_eq!(cpu_time(36_610 * 10_000_000), "10:10:10");
        assert_eq!(uptime((2 * 86_400 + 3 * 3600 + 14 * 60 + 55) * 1000), "2:03:14:55");
        // 2026-11-21 23:59:59 UTC = Unix 1795305599
        let ft = (1_795_305_599u64 + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime(ft, 0), "2026-11-21 23:59:59");
        assert_eq!(filetime(ft, 9 * 60), "2026-11-22 08:59:59");
        assert_eq!(filetime(0, 0), "—");
    }
}
