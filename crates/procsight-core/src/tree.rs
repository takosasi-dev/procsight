//! 親子の組み立てと、画面に出す順の行の並び(FR-2・FR-4・FR-5)。
//! 親は diff が開始時刻で使い回しを除いて決めた `ProcessRow::parent` を使う。

use crate::model::*;
use std::collections::{HashMap, HashSet};

/// 表に出す 1 行。`index` は `Snapshot::rows` の位置
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleRow {
    pub index: usize,
    pub depth: u16,
    pub has_children: bool,
    pub expanded: bool,
    /// 子が絞り込みに一致したので残した親(弱い文字で出す)
    pub dimmed: bool,
}

/// rows の位置で表した森
#[derive(Clone, Debug, Default)]
pub struct Forest {
    pub roots: Vec<usize>,
    pub children: Vec<Vec<usize>>,
}

pub fn build(rows: &[ProcessRow]) -> Forest {
    let pos: HashMap<ProcId, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    let mut f = Forest { roots: Vec::new(), children: vec![Vec::new(); rows.len()] };
    for (i, r) in rows.iter().enumerate() {
        match r.parent.and_then(|p| pos.get(&p)) {
            Some(&p) if p != i && !creates_cycle(rows, &pos, i) => f.children[p].push(i),
            _ => f.roots.push(i),
        }
    }
    f
}

/// 親を辿って自分に戻る・深さ 256 を超えるなら輪とみなして根に置く(E-7)
fn creates_cycle(rows: &[ProcessRow], pos: &HashMap<ProcId, usize>, start: usize) -> bool {
    let mut seen = HashSet::new();
    let mut cur = start;
    for _ in 0..256 {
        if !seen.insert(cur) {
            return true;
        }
        match rows[cur].parent.and_then(|p| pos.get(&p)) {
            Some(&p) => {
                if p == start {
                    return true;
                }
                cur = p;
            }
            None => return false,
        }
    }
    true
}

/// ツリーの表示の順に並べる。`cmp` で同じ親を持つ行の中を並べ、`matches` に合う行と、その祖先を残す。
/// 畳んだ行(collapsed)の子孫は出さない
pub fn flatten_tree(
    rows: &[ProcessRow],
    forest: &Forest,
    collapsed: &HashSet<ProcId>,
    matches: &dyn Fn(&ProcessRow) -> bool,
    cmp: &dyn Fn(&ProcessRow, &ProcessRow) -> std::cmp::Ordering,
) -> Vec<VisibleRow> {
    // 部分木のどこかに一致があるか
    let mut keep = vec![false; rows.len()];
    fn mark(i: usize, rows: &[ProcessRow], f: &Forest, m: &dyn Fn(&ProcessRow) -> bool, keep: &mut [bool]) -> bool {
        let mut any = m(&rows[i]);
        for &c in &f.children[i] {
            any |= mark(c, rows, f, m, keep);
        }
        keep[i] = any;
        any
    }
    for &r in &forest.roots {
        mark(r, rows, forest, matches, &mut keep);
    }
    let mut out = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn emit(
        list: &[usize],
        depth: u16,
        rows: &[ProcessRow],
        f: &Forest,
        keep: &[bool],
        collapsed: &HashSet<ProcId>,
        matches: &dyn Fn(&ProcessRow) -> bool,
        cmp: &dyn Fn(&ProcessRow, &ProcessRow) -> std::cmp::Ordering,
        out: &mut Vec<VisibleRow>,
    ) {
        let mut sorted: Vec<usize> = list.iter().copied().filter(|&i| keep[i]).collect();
        sorted.sort_by(|&a, &b| cmp(&rows[a], &rows[b]));
        for i in sorted {
            let kids_kept = f.children[i].iter().any(|&c| keep[c]);
            let expanded = !collapsed.contains(&rows[i].id);
            out.push(VisibleRow {
                index: i,
                depth,
                has_children: kids_kept,
                expanded,
                dimmed: !matches(&rows[i]),
            });
            if expanded && kids_kept {
                emit(&f.children[i], depth.saturating_add(1), rows, f, keep, collapsed, matches, cmp, out);
            }
        }
    }
    emit(&forest.roots, 0, rows, forest, &keep, collapsed, matches, cmp, &mut out);
    out
}

/// 一覧の表示(入れ子なし)
pub fn flatten_list(
    rows: &[ProcessRow],
    matches: &dyn Fn(&ProcessRow) -> bool,
    cmp: &dyn Fn(&ProcessRow, &ProcessRow) -> std::cmp::Ordering,
) -> Vec<VisibleRow> {
    let mut idx: Vec<usize> = (0..rows.len()).filter(|&i| matches(&rows[i])).collect();
    idx.sort_by(|&a, &b| cmp(&rows[a], &rows[b]));
    idx.into_iter()
        .map(|index| VisibleRow { index, depth: 0, has_children: false, expanded: false, dimmed: false })
        .collect()
}

/// 子孫の識別子(自分を含まない)
pub fn descendants(rows: &[ProcessRow], forest: &Forest, index: usize) -> Vec<ProcId> {
    let mut out = Vec::new();
    let mut stack = forest.children[index].clone();
    let mut seen = HashSet::new();
    while let Some(i) = stack.pop() {
        if seen.insert(i) {
            out.push(rows[i].id);
            stack.extend(forest.children[i].iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{diff, tests::proc_};
    use crate::rules::TouchFreeSet;

    fn snap(ps: Vec<RawProcess>) -> Snapshot {
        diff(
            None,
            RawSnapshot { taken_at_ms: 0, logical_cpus: 1, processes: ps },
            &GpuByPid::new(),
            &WindowInfo::default(),
            &TouchFreeSet::default(),
            &StaticCache::new(),
        )
    }

    fn by_pid(a: &ProcessRow, b: &ProcessRow) -> std::cmp::Ordering {
        a.id.pid.cmp(&b.id.pid)
    }

    #[test]
    fn testee_tree_three_levels_thirteen_rows() {
        // AC-3 の形: 根 1、子 3、孫 9
        let mut ps = vec![proc_(100, 1, 0, "procsight-testee.exe")];
        let mut next = 200;
        for c in 0..3 {
            let child = next;
            next += 1;
            ps.push(proc_(child, 2, 100, "procsight-testee.exe"));
            for _ in 0..3 {
                ps.push(proc_(next, 3 + c, child, "procsight-testee.exe"));
                next += 1;
            }
        }
        let s = snap(ps);
        let f = build(&s.rows);
        let v = flatten_tree(&s.rows, &f, &HashSet::new(), &|_| true, &by_pid);
        assert_eq!(v.len(), 13);
        assert_eq!(v.iter().map(|r| r.depth).max(), Some(2));
        assert_eq!(descendants(&s.rows, &f, v[0].index).len(), 12);
    }

    #[test]
    fn collapse_hides_children_and_filter_keeps_parent_dimmed() {
        let s = snap(vec![proc_(10, 1, 0, "explorer.exe"), proc_(20, 2, 10, "chrome.exe"), proc_(30, 3, 0, "x.exe")]);
        let f = build(&s.rows);
        let collapsed = HashSet::from([ProcId { pid: 10, create_time: 1 }]);
        let v = flatten_tree(&s.rows, &f, &collapsed, &|_| true, &by_pid);
        assert_eq!(v.len(), 2);
        assert!(v[0].has_children && !v[0].expanded);
        let v = flatten_tree(&s.rows, &f, &HashSet::new(), &|r| r.name == "chrome.exe", &by_pid);
        assert_eq!(v.len(), 2);
        assert!(v[0].dimmed && !v[1].dimmed);
    }

    #[test]
    fn siblings_sorted_within_parent() {
        let s = snap(vec![
            proc_(10, 1, 0, "p.exe"),
            proc_(5, 2, 10, "c.exe"),
            proc_(30, 2, 10, "b.exe"),
            proc_(1, 0, 0, "top.exe"),
        ]);
        let f = build(&s.rows);
        let v = flatten_tree(&s.rows, &f, &HashSet::new(), &|_| true, &by_pid);
        let pids: Vec<u32> = v.iter().map(|r| s.rows[r.index].id.pid).collect();
        assert_eq!(pids, vec![1, 10, 5, 30]);
    }

    #[test]
    fn cycle_goes_to_root() {
        let mut s = snap(vec![proc_(10, 1, 0, "a.exe"), proc_(20, 1, 0, "b.exe")]);
        s.rows[0].parent = Some(s.rows[1].id);
        s.rows[1].parent = Some(s.rows[0].id);
        let f = build(&s.rows);
        let v = flatten_tree(&s.rows, &f, &HashSet::new(), &|_| true, &by_pid);
        assert_eq!(v.len(), 2, "輪でも全部の行が出る");
    }
}
