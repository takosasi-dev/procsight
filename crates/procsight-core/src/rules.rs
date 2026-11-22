//! 触れないプロセスの判定(D-7・FR-22)と、終了・一時停止を断る判定(D-17・INV-4)。
//! 断る判定は画面の灰色と、Worker が実際に操作する直前の両方で使う。

use crate::model::*;
use std::collections::{HashMap, HashSet};

/// 触れないプロセスに登録できる数の上限(FR-21)
pub const MAX_TOUCH_FREE: usize = 100;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TouchFreeSet {
    /// 小文字の exe 名
    pub names: Vec<String>,
    pub include_descendants: bool,
    /// 前の周に触れないとした識別子。まだ動いていれば触れないのまま残す。
    /// 登録したランチャーが終わった後も、そのランチャーが起動したゲームを触れないのままにするため(INV-1。D-7 を補う)
    pub keep: HashSet<ProcId>,
}

impl TouchFreeSet {
    /// 登録の中身(名前と子孫の設定)が同じか。違えば keep を引き継がない
    pub fn same_rules(&self, other: &TouchFreeSet) -> bool {
        self.names == other.names && self.include_descendants == other.include_descendants
    }

    /// 名前が一致する物と、(設定がオンなら)親を辿って一致する物に行き着く物と、keep のうちまだ動いている物の識別子を返す。
    /// 親は開始時刻で使い回しを除いて辿る。輪になったら・深さ 256 で打ち切る(E-7)
    pub fn resolve(&self, raw: &RawSnapshot) -> HashSet<ProcId> {
        let mut out: HashSet<ProcId> = raw.processes.iter().map(|p| p.id).filter(|id| self.keep.contains(id)).collect();
        if self.names.is_empty() {
            return out;
        }
        let names: HashSet<String> = self.names.iter().map(|n| n.to_lowercase()).collect();
        let hit = |p: &RawProcess| names.contains(&p.image_name.to_lowercase());
        let by_pid: HashMap<u32, &RawProcess> = raw.processes.iter().map(|p| (p.id.pid, p)).collect();
        for p in &raw.processes {
            if hit(p) {
                out.insert(p.id);
                continue;
            }
            if !self.include_descendants {
                continue;
            }
            let mut cur = p;
            let mut visited = HashSet::new();
            for _ in 0..256 {
                if !visited.insert(cur.id.pid) || cur.parent_pid == cur.id.pid || cur.id.pid == 0 {
                    break;
                }
                let Some(parent) = by_pid.get(&cur.parent_pid) else { break };
                if parent.id.create_time > cur.id.create_time {
                    break; // 親の PID が使い回されている
                }
                if hit(parent) {
                    out.insert(p.id);
                    break;
                }
                cur = parent;
            }
        }
        out
    }
}

/// 入力を exe 名に直す(FR-21・E-39)。パスはファイル名の部分だけ、小文字、前後の空白を除く。空なら None
pub fn normalize_touch_free_name(input: &str) -> Option<String> {
    let name = input.trim().rsplit(['\\', '/']).next().unwrap_or("").trim().to_lowercase();
    (!name.is_empty()).then_some(name)
}

/// 1 行に 1 つの入力をまとめて直す。空と重複を捨て、最大 100 個
pub fn normalize_touch_free_list<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if let Some(n) = normalize_touch_free_name(line)
            && !out.contains(&n)
        {
            out.push(n);
        }
        if out.len() == MAX_TOUCH_FREE {
            break;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    TouchFree,
    Critical,
    CoreProcess(&'static str),
    CannotVerify(&'static str),
    SelfProcess,
    Gone,
}

impl Refusal {
    /// §9.12 の文言
    pub fn message(self) -> &'static str {
        match self {
            Refusal::TouchFree => "触れないプロセスに登録されています",
            Refusal::Critical => "Windows の動作に欠かせないプロセスのため、終了・一時停止できません",
            Refusal::CoreProcess(_) => "Windows の中核のプロセスのため、終了・一時停止できません",
            Refusal::CannotVerify(_) => {
                "Windows の中核のプロセスか確かめられないため、止めました。管理者で再起動すると確かめられます"
            }
            Refusal::SelfProcess => "ProcSight 自身は、メニューの「終了」で閉じてください",
            Refusal::Gone => "すでに終了しています",
        }
    }
}

/// パスを持たない中核の擬似プロセス(INV-4)
const PSEUDO: [&str; 3] = ["Registry", "Memory Compression", "Secure System"];
/// System32 の下にあるときに断る名前(INV-4・D-17)
const CORE: [&str; 6] = ["smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe"];

fn under_system32(path: &str, system_root: &str) -> bool {
    let dir = format!("{}\\system32\\", system_root.trim_end_matches(['\\', '/']).to_lowercase());
    let p = path.to_lowercase().replace('/', "\\");
    p.starts_with(&dir) && !p[dir.len()..].contains('\\')
}

fn check_common(row: &ProcessRow, self_pid: u32, system_root: &str, extra: &[&'static str]) -> Result<(), Refusal> {
    if row.touch_free {
        return Err(Refusal::TouchFree);
    }
    if row.is_gone > 0 {
        return Err(Refusal::Gone);
    }
    if row.id.pid == 0 {
        return Err(Refusal::CoreProcess("System Idle Process"));
    }
    if row.id.pid == 4 {
        return Err(Refusal::CoreProcess("System"));
    }
    if let Some(n) = PSEUDO.iter().find(|n| **n == row.name) {
        return Err(Refusal::CoreProcess(n));
    }
    if row.id.pid == self_pid {
        return Err(Refusal::SelfProcess);
    }
    if row.static_info.as_ref().and_then(|s| s.critical) == Some(true) {
        return Err(Refusal::Critical);
    }
    let lower = row.name.to_lowercase();
    if let Some(n) = CORE.iter().chain(extra).find(|n| **n == lower) {
        return match row.path() {
            Some(path) if under_system32(path, system_root) => Err(Refusal::CoreProcess(n)),
            Some(_) => Ok(()),
            None => Err(Refusal::CannotVerify(n)),
        };
    }
    Ok(())
}

pub fn check_terminate(row: &ProcessRow, self_pid: u32, system_root: &str) -> Result<(), Refusal> {
    check_common(row, self_pid, system_root, &[])
}

/// 一時停止は、終了の判定に加えて dwm.exe も断る(INV-4)
pub fn check_suspend(row: &ProcessRow, self_pid: u32, system_root: &str) -> Result<(), Refusal> {
    check_common(row, self_pid, system_root, &["dwm.exe"])
}

/// ツリーの終了の対象を集める(FR-11・E-16)。葉から順に並べる。
/// 断った物とその子孫は除外し、理由を添えて返す
pub fn collect_tree_for_terminate(
    snap: &Snapshot,
    root: ProcId,
    self_pid: u32,
    system_root: &str,
) -> (Vec<ProcId>, Vec<(ProcId, Refusal)>) {
    let mut children: HashMap<ProcId, Vec<ProcId>> = HashMap::new();
    for r in snap.rows.iter().filter(|r| r.is_gone == 0) {
        if let Some(p) = r.parent {
            children.entry(p).or_default().push(r.id);
        }
    }
    let mut kill = Vec::new();
    let mut excluded = Vec::new();
    // 深さ優先で後順(子を先に)に並べる。輪にならないよう訪れた物を覚える
    #[allow(clippy::too_many_arguments)]
    fn walk(
        id: ProcId,
        snap: &Snapshot,
        children: &HashMap<ProcId, Vec<ProcId>>,
        self_pid: u32,
        root_dir: &str,
        seen: &mut HashSet<ProcId>,
        kill: &mut Vec<ProcId>,
        excluded: &mut Vec<(ProcId, Refusal)>,
    ) {
        if !seen.insert(id) || seen.len() > 10_000 {
            return;
        }
        let Some(row) = snap.find(id) else { return };
        if let Err(r) = check_terminate(row, self_pid, root_dir) {
            excluded.push((id, r)); // 外した物の子孫も外す
            return;
        }
        if let Some(kids) = children.get(&id) {
            for k in kids {
                walk(*k, snap, children, self_pid, root_dir, seen, kill, excluded);
            }
        }
        kill.push(id);
    }
    let mut seen = HashSet::new();
    walk(root, snap, &children, self_pid, system_root, &mut seen, &mut kill, &mut excluded);
    (kill, excluded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::tests::proc_;
    use std::sync::Arc;

    fn raw(ps: Vec<RawProcess>) -> RawSnapshot {
        RawSnapshot { taken_at_ms: 0, logical_cpus: 1, processes: ps }
    }

    #[test]
    fn touch_free_matches_name_and_descendants() {
        let r = raw(vec![
            proc_(10, 1, 0, "Game.exe"),
            proc_(20, 2, 10, "helper.exe"),
            proc_(30, 3, 20, "grandchild.exe"),
            proc_(40, 0, 10, "reused.exe"), // 親より古い → 使い回し
            proc_(50, 5, 0, "other.exe"),
        ]);
        let set = TouchFreeSet { names: vec!["game.exe".into()], include_descendants: true, ..Default::default() };
        let ids: HashSet<u32> = set.resolve(&r).iter().map(|i| i.pid).collect();
        assert_eq!(ids, HashSet::from([10, 20, 30]));
        let only = TouchFreeSet { names: vec!["game.exe".into()], include_descendants: false, ..Default::default() };
        let ids: HashSet<u32> = only.resolve(&r).iter().map(|i| i.pid).collect();
        assert_eq!(ids, HashSet::from([10]));
    }

    #[test]
    fn touch_free_kept_after_launcher_exits() {
        let set = TouchFreeSet { names: vec!["launcher.exe".into()], include_descendants: true, ..Default::default() };
        let first = set.resolve(&raw(vec![proc_(10, 1, 0, "launcher.exe"), proc_(20, 2, 10, "game.exe")]));
        assert_eq!(first.len(), 2);
        // ランチャーが終わった。keep があればゲームは触れないのまま、無ければ外れる
        let later = raw(vec![proc_(20, 2, 10, "game.exe"), proc_(30, 3, 0, "other.exe")]);
        assert!(set.resolve(&later).is_empty());
        let kept = TouchFreeSet { keep: first, ..set.clone() };
        let ids: HashSet<u32> = kept.resolve(&later).iter().map(|i| i.pid).collect();
        assert_eq!(ids, HashSet::from([20]));
        assert!(kept.same_rules(&set));
    }

    #[test]
    fn touch_free_survives_parent_cycle() {
        let r = raw(vec![proc_(10, 1, 20, "a.exe"), proc_(20, 1, 10, "b.exe")]);
        let set = TouchFreeSet { names: vec!["x.exe".into()], include_descendants: true, ..Default::default() };
        assert!(set.resolve(&r).is_empty());
    }

    #[test]
    fn normalize_names() {
        assert_eq!(normalize_touch_free_name("  C:\\Games\\Ace8\\ACE8.EXE "), Some("ace8.exe".into()));
        assert_eq!(normalize_touch_free_name("game"), Some("game".into()));
        assert_eq!(normalize_touch_free_name("   "), None);
        let list = normalize_touch_free_list(["a.exe", "A.EXE", "", "b.exe"]);
        assert_eq!(list, vec!["a.exe", "b.exe"]);
        let many: Vec<String> = (0..150).map(|i| format!("{i}.exe")).collect();
        assert_eq!(normalize_touch_free_list(many.iter().map(|s| s.as_str())).len(), 100);
    }

    fn row(pid: u32, name: &str, path: Option<&str>, critical: Option<bool>) -> ProcessRow {
        let s = crate::diff::diff(
            None,
            raw(vec![proc_(pid, 1, 0, name)]),
            &GpuByPid::new(),
            &WindowInfo::default(),
            &TouchFreeSet::default(),
            &StaticCache::new(),
        );
        let mut r = s.rows[0].clone();
        r.static_info = Some(Arc::new(StaticInfo { path: path.map(Into::into), critical, ..Default::default() }));
        r
    }

    const ROOT: &str = "C:\\Windows";

    #[test]
    fn refuses_core_processes() {
        assert_eq!(check_terminate(&row(0, "", None, None), 1, ROOT), Err(Refusal::CoreProcess("System Idle Process")));
        assert_eq!(check_terminate(&row(4, "System", None, None), 1, ROOT), Err(Refusal::CoreProcess("System")));
        assert!(matches!(check_terminate(&row(90, "Registry", None, None), 1, ROOT), Err(Refusal::CoreProcess(_))));
        assert!(matches!(
            check_terminate(&row(5, "csrss.exe", Some("C:\\Windows\\System32\\csrss.exe"), None), 1, ROOT),
            Err(Refusal::CoreProcess("csrss.exe"))
        ));
        assert!(matches!(
            check_terminate(&row(5, "LSASS.EXE", Some("c:\\windows\\system32\\lsass.exe"), None), 1, ROOT),
            Err(Refusal::CoreProcess(_))
        ));
        // 別の場所の csrss.exe は止められる(D-17)
        assert_eq!(check_terminate(&row(5, "csrss.exe", Some("C:\\Users\\x\\csrss.exe"), None), 1, ROOT), Ok(()));
        assert_eq!(
            check_terminate(&row(5, "csrss.exe", Some("C:\\Windows\\System32\\sub\\csrss.exe"), None), 1, ROOT),
            Ok(())
        );
        // パスが取れなければ安全側に倒す
        assert_eq!(check_terminate(&row(5, "winlogon.exe", None, None), 1, ROOT), Err(Refusal::CannotVerify("winlogon.exe")));
        assert_eq!(check_terminate(&row(7, "foo.exe", None, Some(true)), 1, ROOT), Err(Refusal::Critical));
        assert_eq!(check_terminate(&row(7, "foo.exe", None, None), 7, ROOT), Err(Refusal::SelfProcess));
        assert_eq!(check_terminate(&row(7, "foo.exe", None, None), 1, ROOT), Ok(()));
    }

    #[test]
    fn suspend_also_refuses_dwm() {
        let dwm = row(8, "dwm.exe", Some("C:\\Windows\\System32\\dwm.exe"), None);
        assert_eq!(check_terminate(&dwm, 1, ROOT), Ok(()));
        assert_eq!(check_suspend(&dwm, 1, ROOT), Err(Refusal::CoreProcess("dwm.exe")));
    }

    #[test]
    fn touch_free_and_gone_refused() {
        let mut r = row(9, "foo.exe", None, None);
        r.touch_free = true;
        assert_eq!(check_terminate(&r, 1, ROOT), Err(Refusal::TouchFree));
        let mut g = row(9, "foo.exe", None, None);
        g.is_gone = 1;
        assert_eq!(check_suspend(&g, 1, ROOT), Err(Refusal::Gone));
    }

    #[test]
    fn tree_terminate_order_and_exclusion() {
        let s = crate::diff::diff(
            None,
            raw(vec![
                proc_(10, 1, 0, "root.exe"),
                proc_(20, 2, 10, "a.exe"),
                proc_(21, 2, 20, "a1.exe"),
                proc_(30, 3, 10, "game.exe"),
                proc_(31, 4, 30, "under-game.exe"),
            ]),
            &GpuByPid::new(),
            &WindowInfo::default(),
            &TouchFreeSet { names: vec!["game.exe".into()], include_descendants: false, ..Default::default() },
            &StaticCache::new(),
        );
        let (kill, excluded) = collect_tree_for_terminate(&s, ProcId { pid: 10, create_time: 1 }, 1, ROOT);
        let pids: Vec<u32> = kill.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![21, 20, 10], "葉から");
        assert_eq!(excluded, vec![(ProcId { pid: 30, create_time: 3 }, Refusal::TouchFree)]);
    }
}
