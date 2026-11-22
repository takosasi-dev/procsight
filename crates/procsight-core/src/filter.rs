//! 絞り込み(FR-5)と並べ替え(FR-4)。値の無い行は昇順・降順のどちらでも最後、同じ値なら PID の昇順。

use crate::model::*;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scope {
    #[default]
    All,
    WithWindow,
    MyUser,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::All, Scope::WithWindow, Scope::MyUser];
    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "すべて",
            Scope::WithWindow => "ウィンドウを持つ物",
            Scope::MyUser => "自分のユーザーの物",
        }
    }
}

/// 入力は前後の空白を除き、大文字・小文字を区別しない部分一致。数字だけなら PID の完全一致も残す
pub fn matches(row: &ProcessRow, query: &str, scope: Scope, my_user: Option<&str>) -> bool {
    let in_scope = match scope {
        Scope::All => true,
        Scope::WithWindow => row.has_window,
        Scope::MyUser => match (row.user(), my_user) {
            (Some(u), Some(me)) => u.eq_ignore_ascii_case(me),
            _ => false,
        },
    };
    if !in_scope {
        return false;
    }
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    if q.chars().all(|c| c.is_ascii_digit()) && q.parse::<u32>() == Ok(row.id.pid) {
        return true;
    }
    row.name.to_lowercase().contains(&q) || row.path().is_some_and(|p| p.to_lowercase().contains(&q))
}

/// 並べ替えに使う値
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum SortValue {
    Num(f64),
    Text(String),
    None,
}

/// 行だけから決まる列の値。署名の列は行に無いので、画面が別に渡す
pub fn column_value(row: &ProcessRow, col: Column) -> SortValue {
    use SortValue::*;
    let num = |v: Option<f64>| v.map_or(None, Num);
    match col {
        Column::Name => Text(row.name.to_lowercase()),
        Column::Pid => Num(row.id.pid as f64),
        Column::State => Text(row.state.label().into()),
        Column::Cpu => num(row.cpu_pct.map(f64::from)),
        Column::Mem => num(row.mem.map(|v| v as f64)),
        Column::WorkingSet => Num(row.working_set as f64),
        Column::Commit => Num(row.commit as f64),
        Column::IoRead => num(row.io_read_bps),
        Column::IoWrite => num(row.io_write_bps),
        Column::Gpu => num(row.gpu_pct.map(f64::from)),
        Column::Threads => Num(row.threads as f64),
        Column::Handles => Num(row.handles as f64),
        Column::User => row.user().map_or(None, |u| Text(u.to_lowercase())),
        Column::Priority => Num(row.base_priority as f64),
        Column::Session => Num(row.session_id as f64),
        Column::StartTime => Num(row.id.create_time as f64),
        Column::Path => row.path().map_or(None, |p| Text(p.to_lowercase())),
        Column::Signature => None,
        Column::ParentPid => Num(row.parent_pid as f64),
    }
}

/// 2 つの値を比べる。None は向きによらず後ろ、同じなら PID の昇順
pub fn compare_values(a: &SortValue, b: &SortValue, descending: bool, pid_a: u32, pid_b: u32) -> Ordering {
    let ord = match (a, b) {
        (SortValue::None, SortValue::None) => Ordering::Equal,
        (SortValue::None, _) => return Ordering::Greater,
        (_, SortValue::None) => return Ordering::Less,
        (SortValue::Num(x), SortValue::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (SortValue::Text(x), SortValue::Text(y)) => x.cmp(y),
        (SortValue::Num(_), _) => Ordering::Less,
        (_, SortValue::Num(_)) => Ordering::Greater,
    };
    let ord = if descending { ord.reverse() } else { ord };
    ord.then(pid_a.cmp(&pid_b))
}

pub fn compare_rows(a: &ProcessRow, b: &ProcessRow, col: Column, descending: bool) -> Ordering {
    compare_values(&column_value(a, col), &column_value(b, col), descending, a.id.pid, b.id.pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{diff, tests::proc_};
    use crate::rules::TouchFreeSet;
    use std::sync::Arc;

    fn rows() -> Vec<ProcessRow> {
        let s = diff(
            None,
            RawSnapshot {
                taken_at_ms: 0,
                logical_cpus: 1,
                processes: vec![proc_(30, 1, 0, "procsight-testee.exe"), proc_(4, 1, 0, "System"), proc_(77, 1, 0, "b.exe")],
            },
            &GpuByPid::new(),
            &WindowInfo::default(),
            &TouchFreeSet::default(),
            &StaticCache::new(),
        );
        let mut r = s.rows;
        r[0].cpu_pct = Some(10.0);
        r[1].cpu_pct = None;
        r[2].cpu_pct = Some(10.0);
        r[2].static_info = Some(Arc::new(StaticInfo {
            path: Some("C:\\Tools\\B.exe".into()),
            user: Some("Me".into()),
            ..Default::default()
        }));
        r[0].has_window = true;
        r
    }

    #[test]
    fn match_name_pid_path_case_insensitive() {
        let r = rows();
        assert!(matches(&r[0], "TESTEE", Scope::All, None));
        assert!(!matches(&r[1], "TESTEE", Scope::All, None));
        assert!(matches(&r[2], "tools", Scope::All, None), "パスの部分一致");
        assert!(matches(&r[0], "30", Scope::All, None), "PID の完全一致");
        assert!(!matches(&r[0], "3", Scope::All, None));
        assert!(matches(&r[1], " ", Scope::All, None));
    }

    #[test]
    fn scopes() {
        let r = rows();
        assert!(matches(&r[0], "", Scope::WithWindow, None));
        assert!(!matches(&r[2], "", Scope::WithWindow, None));
        assert!(matches(&r[2], "", Scope::MyUser, Some("me")));
        assert!(!matches(&r[0], "", Scope::MyUser, Some("me")), "ユーザーが取れない行は外す");
    }

    #[test]
    fn sort_none_last_and_pid_tiebreak() {
        let r = rows();
        let mut desc: Vec<&ProcessRow> = r.iter().collect();
        desc.sort_by(|a, b| compare_rows(a, b, Column::Cpu, true));
        assert_eq!(desc.iter().map(|x| x.id.pid).collect::<Vec<_>>(), vec![30, 77, 4]);
        let mut asc: Vec<&ProcessRow> = r.iter().collect();
        asc.sort_by(|a, b| compare_rows(a, b, Column::Cpu, false));
        assert_eq!(asc.iter().map(|x| x.id.pid).collect::<Vec<_>>(), vec![30, 77, 4]);
        let mut by_name: Vec<&ProcessRow> = r.iter().collect();
        by_name.sort_by(|a, b| compare_rows(a, b, Column::Name, false));
        assert_eq!(by_name[0].name, "b.exe");
    }
}
