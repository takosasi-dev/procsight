//! Ctrl+Shift+Esc の置き換え(FR-53〜FR-56)の判定。レジストリの読み書きは procsight-win の
//! taskmgr_replace.rs だけが行い(INV-13)、ここでは「オンにしてよい場所か」「書く値」「起動の引数」を決める。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceBlock {
    NotElevated,
    /// 実体のパスが %ProgramFiles% の下でない、または WindowsApps の下(D-39)
    Location,
}

impl ReplaceBlock {
    pub fn message(self) -> &'static str {
        match self {
            ReplaceBlock::NotElevated => "管理者で再起動すると変えられます",
            ReplaceBlock::Location => "ProcSight.exe を C:\\Program Files\\ の下に置いたときだけ使えます",
        }
    }
}

fn norm(p: &str) -> String {
    let p = p.trim().replace('/', "\\").to_lowercase();
    // GetFinalPathNameByHandleW は \\?\ を付けて返す
    p.strip_prefix("\\\\?\\").map(str::to_string).unwrap_or(p)
}

/// D-39。`final_path` は ProcSight.exe の実体のパス(リンクを辿った後)、`program_files` は %ProgramFiles%
pub fn check_location(final_path: &str, program_files: &str) -> Result<(), ReplaceBlock> {
    let p = norm(final_path);
    let pf = format!("{}\\", norm(program_files).trim_end_matches('\\'));
    if pf.len() < 4 || !p.starts_with(&pf) || p.starts_with(&format!("{pf}windowsapps\\")) || p.contains("\\..\\") {
        return Err(ReplaceBlock::Location);
    }
    Ok(())
}

pub fn check_can_enable(elevated: bool, final_path: &str, program_files: &str) -> Result<(), ReplaceBlock> {
    if !elevated {
        return Err(ReplaceBlock::NotElevated);
    }
    check_location(final_path, program_files)
}

/// Debugger に書く値(INV-15)。引用符で囲んだ実体のパスだけ。\\?\ は外す
pub fn debugger_value(final_path: &str) -> String {
    let p = final_path.trim();
    let p = p.strip_prefix("\\\\?\\").unwrap_or(p);
    format!("\"{p}\"")
}

/// Debugger の値が指す exe のパス(引用符と引数を除く)
pub fn debugger_target(value: &str) -> Option<String> {
    crate::startup::exe_from_command(value, &|s| s.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceStatus {
    /// 値が無い
    Off,
    /// ProcSight.exe を指している(今の exe と同じ場所か)
    OnSelf { same_path: bool, target: String },
    /// 別の道具(Process Explorer など)の値
    Other(String),
}

/// 今の Debugger の値から状態を決める(FR-56・E-44・E-46)
pub fn status_of(value: Option<&str>, current_final_path: &str) -> ReplaceStatus {
    let Some(v) = value.filter(|v| !v.trim().is_empty()) else { return ReplaceStatus::Off };
    match debugger_target(v) {
        Some(t) if crate::startup::file_name(&t).eq_ignore_ascii_case("procsight.exe") => {
            let same_path = norm(&t) == norm(current_final_path);
            ReplaceStatus::OnSelf { same_path, target: t }
        }
        _ => ReplaceStatus::Other(v.to_string()),
    }
}

/// 起動の引数(§9.10)。最初の引数を除いた残り(プログラム名の後)を渡す
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchArgs {
    Normal,
    /// D-18。旧 PID が終わるのを待つ
    AfterRestart(u32),
    /// D-43。置き換えからの起動(残りの引数は無視)
    FromTaskmgr,
    /// 知らない引数(ログに警告を書いて普段どおり起動)
    Unknown(Vec<String>),
}

pub fn parse_args(args: &[String]) -> LaunchArgs {
    let Some(first) = args.first() else { return LaunchArgs::Normal };
    if crate::startup::file_name(first.trim_matches('"')).eq_ignore_ascii_case("taskmgr.exe") {
        return LaunchArgs::FromTaskmgr;
    }
    if first == "--after-restart"
        && args.len() == 2
        && let Ok(pid) = args[1].parse()
    {
        return LaunchArgs::AfterRestart(pid);
    }
    LaunchArgs::Unknown(args.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PF: &str = "C:\\Program Files";

    #[test]
    fn location_rules() {
        assert_eq!(check_location("C:\\Program Files\\ProcSight\\ProcSight.exe", PF), Ok(()));
        assert_eq!(check_location("\\\\?\\C:\\PROGRAM FILES\\ProcSight\\ProcSight.exe", PF), Ok(()));
        assert_eq!(check_location("C:\\Users\\me\\Desktop\\ProcSight.exe", PF), Err(ReplaceBlock::Location));
        assert_eq!(check_location("C:\\Program Files (x86)\\ProcSight.exe", PF), Err(ReplaceBlock::Location));
        assert_eq!(
            check_location("C:\\Program Files\\WindowsApps\\x\\ProcSight.exe", PF),
            Err(ReplaceBlock::Location)
        );
        assert_eq!(check_location("C:\\Program Files\\..\\Temp\\ProcSight.exe", PF), Err(ReplaceBlock::Location));
        assert_eq!(check_location("C:\\Program Files\\a.exe", ""), Err(ReplaceBlock::Location));
        assert_eq!(
            check_can_enable(false, "C:\\Program Files\\ProcSight\\ProcSight.exe", PF),
            Err(ReplaceBlock::NotElevated)
        );
    }

    #[test]
    fn value_and_status() {
        let me = "\\\\?\\C:\\Program Files\\ProcSight\\ProcSight.exe";
        assert_eq!(debugger_value(me), "\"C:\\Program Files\\ProcSight\\ProcSight.exe\"");
        assert_eq!(status_of(None, me), ReplaceStatus::Off);
        assert_eq!(status_of(Some(""), me), ReplaceStatus::Off);
        assert!(matches!(status_of(Some(&debugger_value(me)), me), ReplaceStatus::OnSelf { same_path: true, .. }));
        assert!(matches!(
            status_of(Some("\"C:\\Program Files\\ProcSight2\\ProcSight.exe\""), me),
            ReplaceStatus::OnSelf { same_path: false, .. }
        ));
        assert_eq!(
            status_of(Some("\"C:\\Sysinternals\\PROCEXP64.EXE\""), me),
            ReplaceStatus::Other("\"C:\\Sysinternals\\PROCEXP64.EXE\"".into())
        );
    }

    #[test]
    fn args() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&[]), LaunchArgs::Normal);
        assert_eq!(parse_args(&s(&["C:\\WINDOWS\\system32\\Taskmgr.exe", "/4"])), LaunchArgs::FromTaskmgr);
        assert_eq!(parse_args(&s(&["\"C:\\Windows\\System32\\taskmgr.exe\""])), LaunchArgs::FromTaskmgr);
        assert_eq!(parse_args(&s(&["--after-restart", "1234"])), LaunchArgs::AfterRestart(1234));
        assert!(matches!(parse_args(&s(&["--after-restart", "x"])), LaunchArgs::Unknown(_)));
        assert!(matches!(parse_args(&s(&["--kill", "4"])), LaunchArgs::Unknown(_)));
    }
}
