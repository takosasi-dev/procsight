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

/// 範囲(すべて・窓を持つ物・自分のユーザーの物)に入るか
pub fn in_scope(row: &ProcessRow, scope: Scope, my_user: Option<&str>) -> bool {
    match scope {
        Scope::All => true,
        Scope::WithWindow => row.has_window,
        Scope::MyUser => match (row.user(), my_user) {
            (Some(u), Some(me)) => u.eq_ignore_ascii_case(me),
            _ => false,
        },
    }
}

/// 範囲と絞り込みの両方。毎回 parse するので、表のように何度も呼ぶ所は Query を持っておく
pub fn matches(row: &ProcessRow, query: &str, scope: Scope, my_user: Option<&str>) -> bool {
    in_scope(row, scope, my_user) && Query::parse(query).matches(row)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
}

impl Cmp {
    fn test(self, v: f64, x: f64) -> bool {
        match self {
            Cmp::Gt => v > x,
            Cmp::Ge => v >= x,
            Cmp::Lt => v < x,
            Cmp::Le => v <= x,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Metric {
    /// %
    Cpu,
    /// バイト
    Mem,
    /// %
    Gpu,
    /// 読み取り+書き込みのバイト毎秒
    Io,
}

/// 絞り込みの 1 語
#[derive(Clone, Debug, PartialEq)]
enum Term {
    /// 小文字。名前・パスの部分一致、数字だけなら PID の完全一致も
    Text(String),
    Num(Metric, Cmp, f64),
    Pid(u32),
    /// 小文字。ユーザーの部分一致
    User(String),
    State(ProcState),
}

/// 絞り込みの欄の文字を解析した物(FR-5 の拡張)。空白で区切った語を全部満たす行を残す
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    terms: Vec<Term>,
}

/// 「?」の印の説明に出す書き方の一覧(書き方, 意味)
pub const QUERY_HELP: [(&str, &str); 8] = [
    ("note", "名前・パスの一部(数字だけなら PID も)"),
    ("cpu>5", "CPU が 5% より上(>= < <= も使えます)"),
    ("mem>500mb", "メモリ(kb・mb・gb。単位が無ければ MB)"),
    ("gpu>10", "GPU が 10% より上"),
    ("io>1mb", "I/O の読み取り+書き込みが毎秒 1 MB より上"),
    ("pid:1234", "PID が 1234"),
    ("user:system", "ユーザーの名前の一部"),
    ("state:応答なし", "状態(running・suspended・hung・exiting・touchfree でも)"),
];

const KB: f64 = 1024.0;

/// 表に出ている書き方(`1.2 MB/s` を写した `1.2mb/s`)と短い単位(`1g`・`500m`・`64k`)も受ける
fn parse_bytes(s: &str) -> Option<f64> {
    let s = s.strip_suffix("/s").unwrap_or(s);
    let s = s.strip_suffix('b').unwrap_or(s);
    let (num, mul) = if let Some(n) = s.strip_suffix('k') {
        (n, KB)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, KB * KB)
    } else if let Some(n) = s.strip_suffix('g') {
        (n, KB * KB * KB)
    } else {
        (s, KB * KB)
    };
    parse_num(num).map(|v| v * mul)
}

/// 記号(`>`・`<`・`=`・`:`)の前後の空白を詰める。`cpu > 5` や `pid: 1234` を 1 語にする
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        let sym = matches!(c, '>' | '<' | '=' | ':');
        let last_sym = out.ends_with(['>', '<', '=', ':']);
        if pending_space && !out.is_empty() && !sym && !last_sym {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out
}

fn parse_num(s: &str) -> Option<f64> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn parse_state(s: &str) -> Option<ProcState> {
    use ProcState::*;
    let all = [Running, Suspended, NotResponding, Exiting, TouchFree];
    if let Some(st) = all.into_iter().find(|x| x.label() == s) {
        return Some(st);
    }
    Some(match s {
        "running" => Running,
        "suspended" => Suspended,
        "hung" => NotResponding,
        "exiting" => Exiting,
        "touchfree" => TouchFree,
        _ => return None,
    })
}

fn parse_term(word: &str) -> Term {
    let w = word.to_lowercase();
    if let Some(v) = w.strip_prefix("pid:") {
        if let Ok(p) = v.parse::<u32>() {
            return Term::Pid(p);
        }
    } else if let Some(v) = w.strip_prefix("user:") {
        if !v.is_empty() {
            return Term::User(v.to_string());
        }
    } else if let Some(v) = w.strip_prefix("state:") {
        if let Some(s) = parse_state(v) {
            return Term::State(s);
        }
    } else {
        for (key, metric) in [("cpu", Metric::Cpu), ("mem", Metric::Mem), ("gpu", Metric::Gpu), ("io", Metric::Io)] {
            let Some(rest) = w.strip_prefix(key) else { continue };
            let (cmp, val) = if let Some(v) = rest.strip_prefix(">=") {
                (Cmp::Ge, v)
            } else if let Some(v) = rest.strip_prefix("<=") {
                (Cmp::Le, v)
            } else if let Some(v) = rest.strip_prefix('>') {
                (Cmp::Gt, v)
            } else if let Some(v) = rest.strip_prefix('<') {
                (Cmp::Lt, v)
            } else {
                break;
            };
            let x = match metric {
                Metric::Cpu | Metric::Gpu => parse_num(val.strip_suffix('%').unwrap_or(val)),
                Metric::Mem | Metric::Io => parse_bytes(val),
            };
            if let Some(x) = x {
                return Term::Num(metric, cmp, x);
            }
            break;
        }
    }
    Term::Text(w)
}

impl Query {
    /// 形の正しくない語(`cpu>abc`・`state:foo` など)は普通の語として扱う
    pub fn parse(s: &str) -> Query {
        Query { terms: squeeze(s).split_whitespace().map(parse_term).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// 値が取れない行(None)は、どの比較にも一致しない
    pub fn matches(&self, row: &ProcessRow) -> bool {
        self.terms.iter().all(|t| match t {
            Term::Text(q) => {
                (q.chars().all(|c| c.is_ascii_digit()) && q.parse::<u32>() == Ok(row.id.pid))
                    || row.name.to_lowercase().contains(q.as_str())
                    || row.path().is_some_and(|p| p.to_lowercase().contains(q.as_str()))
            }
            Term::Num(m, cmp, x) => {
                let v = match m {
                    Metric::Cpu => row.cpu_pct.map(f64::from),
                    Metric::Mem => row.mem.map(|v| v as f64),
                    Metric::Gpu => row.gpu_pct.map(f64::from),
                    Metric::Io => match (row.io_read_bps, row.io_write_bps) {
                        (None, None) => None,
                        (r, w) => Some(r.unwrap_or(0.0) + w.unwrap_or(0.0)),
                    },
                };
                v.is_some_and(|v| cmp.test(v, *x))
            }
            Term::Pid(p) => row.id.pid == *p,
            Term::User(u) => row.user().is_some_and(|x| x.to_lowercase().contains(u.as_str())),
            Term::State(s) => row.state == *s,
        })
    }
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
        // 推移の列は今の CPU で並べる
        Column::CpuHistory => num(row.cpu_pct.map(f64::from)),
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
    fn query_words_are_and_and_metrics_compare() {
        let mut r = rows();
        r[0].mem = Some(600 * 1024 * 1024);
        r[0].gpu_pct = Some(12.0);
        r[0].io_read_bps = Some(800.0 * 1024.0);
        r[0].io_write_bps = Some(300.0 * 1024.0);
        r[2].mem = Some(10 * 1024 * 1024);
        r[2].state = ProcState::NotResponding;
        let q = Query::parse;
        // AND
        assert!(q("testee cpu>5").matches(&r[0]));
        assert!(!q("testee cpu>50").matches(&r[0]));
        assert!(!q("testee b.exe").matches(&r[0]));
        // 比べ方と単位
        assert!(q("cpu>=10").matches(&r[0]) && !q("cpu>10").matches(&r[0]));
        assert!(q("cpu<=10%").matches(&r[2]) && q("CPU<11").matches(&r[2]));
        assert!(q("mem>500mb").matches(&r[0]) && q("mem>500").matches(&r[0]) && !q("mem>1gb").matches(&r[0]));
        assert!(q("mem<20000kb").matches(&r[2]) && q("mem<0.5gb").matches(&r[2]));
        assert!(q("gpu>10").matches(&r[0]) && !q("gpu>10").matches(&r[2]), "GPU が無い行は外す");
        assert!(q("io>1mb").matches(&r[0]) && !q("io>2mb").matches(&r[0]));
        // 値の無い行(CPU が None)は比べない
        assert!(!q("cpu<1").matches(&r[1]) && !q("cpu>=0").matches(&r[1]));
        // pid・user・state
        assert!(q("pid:77").matches(&r[2]) && !q("pid:7").matches(&r[2]));
        assert!(q("user:ME").matches(&r[2]) && q("user:e").matches(&r[2]) && !q("user:me").matches(&r[0]));
        assert!(q("state:応答なし").matches(&r[2]) && q("state:HUNG").matches(&r[2]) && !q("state:running").matches(&r[2]));
        assert!(q("state:running").matches(&r[0]) && q("state:実行中").matches(&r[0]));
        // 今の呼び出し(範囲と一緒)
        assert!(matches(&r[0], "testee cpu>5", Scope::WithWindow, None));
        assert!(!matches(&r[2], "cpu>5", Scope::WithWindow, None));
    }

    #[test]
    fn malformed_words_are_plain_text() {
        let mut r = rows();
        r[2].name = "cpu>abc state:foo".into();
        for w in ["cpu>abc", "state:foo", "pid:x", "user:", "cpu=5", "mem>5tb", "cpu>", "io>1..2"] {
            assert_eq!(Query::parse(w).terms, vec![Term::Text(w.to_string())], "{w}");
        }
        assert!(Query::parse("cpu>abc state:foo").matches(&r[2]));
        assert!(!Query::parse("cpu>abc").matches(&r[0]));
        assert!(Query::parse("  ").is_empty());
    }

    /// 表から写した単位(MB/s)・短い単位・記号の前後の空白も、数の比べとして読む
    #[test]
    fn lenient_units_and_spaces() {
        let mb = KB * KB;
        assert_eq!(Query::parse("io>1MB/s").terms, vec![Term::Num(Metric::Io, Cmp::Gt, mb)]);
        assert_eq!(Query::parse("mem>1g").terms, vec![Term::Num(Metric::Mem, Cmp::Gt, mb * KB)]);
        assert_eq!(Query::parse("mem>500m").terms, vec![Term::Num(Metric::Mem, Cmp::Gt, 500.0 * mb)]);
        assert_eq!(Query::parse("io>64k").terms, vec![Term::Num(Metric::Io, Cmp::Gt, 64.0 * KB)]);
        assert_eq!(Query::parse("cpu > 5").terms, vec![Term::Num(Metric::Cpu, Cmp::Gt, 5.0)]);
        assert_eq!(Query::parse("cpu >= 5  pid: 4").terms, vec![Term::Num(Metric::Cpu, Cmp::Ge, 5.0), Term::Pid(4)]);
        assert_eq!(Query::parse("foo bar").terms, vec![Term::Text("foo".into()), Term::Text("bar".into())]);
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
