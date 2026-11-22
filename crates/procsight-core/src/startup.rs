//! スタートアップの項目の型と、StartupApproved の値の読み書きの形(仕様書 §9.13・D-26)。
//! 値の形は 2026-11-22 にこの PC の実物で見た: REG_BINARY 12 バイト、先頭の 1 バイトが偶数なら有効
//! (02)、奇数なら無効(03・07)。続く 3 バイトは 0、残り 8 バイトが無効にした日時(FILETIME)。有効は 02 と 0 が 11 個。
//! コマンドの文字列から exe のパスを取り出す処理もここに置く(FR-38・FR-40)。

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupLocation {
    RegistryUser,
    RegistryMachine,
    Registry32,
    FolderUser,
    FolderCommon,
}

impl StartupLocation {
    pub fn label(self) -> &'static str {
        match self {
            StartupLocation::RegistryUser => "レジストリ(ユーザー)",
            StartupLocation::RegistryMachine => "レジストリ(全ユーザー)",
            StartupLocation::Registry32 => "レジストリ(32 ビット)",
            StartupLocation::FolderUser => "スタートアップ フォルダ(ユーザー)",
            StartupLocation::FolderCommon => "スタートアップ フォルダ(全ユーザー)",
        }
    }
    /// 変えるのに管理者が要る場所(FR-39)
    pub fn needs_admin(self) -> bool {
        !matches!(self, StartupLocation::RegistryUser | StartupLocation::FolderUser)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approved {
    Enabled,
    /// 無効にした日時(FILETIME。0 なら不明)
    Disabled(u64),
    /// 長さが 12 でない(E-30)
    Unknown,
}

impl Approved {
    pub fn label(self) -> &'static str {
        match self {
            Approved::Enabled => "有効",
            Approved::Disabled(_) => "無効",
            Approved::Unknown => "不明",
        }
    }
}

pub const APPROVED_LEN: usize = 12;

/// 値が無ければ有効とみなす(D-26)
pub fn decode_approved(value: Option<&[u8]>) -> Approved {
    let Some(v) = value else { return Approved::Enabled };
    if v.len() != APPROVED_LEN {
        return Approved::Unknown;
    }
    if v[0] % 2 == 0 {
        Approved::Enabled
    } else {
        let mut ft = [0u8; 8];
        ft.copy_from_slice(&v[4..12]);
        Approved::Disabled(u64::from_le_bytes(ft))
    }
}

/// タスク マネージャーと同じ形で書く値。無効は 03・0・0・0 と日時、有効は 02 と 0 が 11 個
pub fn encode_approved(enabled: bool, now_filetime: u64) -> [u8; APPROVED_LEN] {
    let mut v = [0u8; APPROVED_LEN];
    if enabled {
        v[0] = 0x02;
    } else {
        v[0] = 0x03;
        v[4..12].copy_from_slice(&now_filetime.to_le_bytes());
    }
    v
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartupItem {
    /// Run の値の名前、またはフォルダのファイル名
    pub name: String,
    pub location: StartupLocation,
    pub command: String,
    pub exe_path: Option<String>,
    pub exe_exists: bool,
    pub approved: Approved,
    /// 署名者(D-23)。exe が分からない・触れないプロセスの名前なら None(「—」)
    pub publisher: Option<String>,
}

/// コマンドから exe のパスを取り出す。引用符で囲まれていればその中、無ければ「.exe」までか最初の空白まで。
/// 環境変数(%SystemRoot% など)は `expand` で展開する
pub fn exe_from_command(command: &str, expand: &dyn Fn(&str) -> String) -> Option<String> {
    let c = command.trim();
    if c.is_empty() {
        return None;
    }
    let raw = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let lower = c.to_lowercase();
        match lower.find(".exe") {
            Some(i) => c[..i + 4].to_string(),
            None => c.split_whitespace().next()?.to_string(),
        }
    };
    let p = expand(raw.trim());
    (!p.is_empty()).then_some(p)
}

/// パスのファイル名の部分
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn decode_real_values() {
        // 2026-11-22 にこの PC で読んだ実物(名前は伏せた)
        assert_eq!(decode_approved(Some(&hex("020000000000000000000000"))), Approved::Enabled);
        let Approved::Disabled(ft) = decode_approved(Some(&hex("0300000005DC64F89D24D801"))) else { panic!() };
        assert_eq!(ft, 0x01D8249DF864DC05);
        assert!(matches!(decode_approved(Some(&hex("0700000029CF1C993CE8D701"))), Approved::Disabled(_)));
        assert_eq!(decode_approved(None), Approved::Enabled);
        assert_eq!(decode_approved(Some(&[2, 0, 0])), Approved::Unknown);
    }

    #[test]
    fn encode_like_task_manager() {
        assert_eq!(encode_approved(true, 12345).to_vec(), hex("020000000000000000000000"));
        let v = encode_approved(false, 0x01D8249DF864DC05);
        assert_eq!(v.to_vec(), hex("0300000005DC64F89D24D801"));
        assert_eq!(decode_approved(Some(&v)), Approved::Disabled(0x01D8249DF864DC05));
    }

    #[test]
    fn command_to_exe() {
        let id = |s: &str| s.replace("%ProgramFiles%", "C:\\Program Files");
        assert_eq!(
            exe_from_command("\"C:\\Program Files\\App\\app.exe\" --tray", &id).as_deref(),
            Some("C:\\Program Files\\App\\app.exe")
        );
        assert_eq!(
            exe_from_command("C:\\Program Files\\App\\App.EXE /min", &id).as_deref(),
            Some("C:\\Program Files\\App\\App.EXE")
        );
        assert_eq!(
            exe_from_command("%ProgramFiles%\\x\\y.exe", &id).as_deref(),
            Some("C:\\Program Files\\x\\y.exe")
        );
        assert_eq!(exe_from_command("rundll32 foo.dll,Bar", &id).as_deref(), Some("rundll32"));
        assert_eq!(exe_from_command("  ", &id), None);
        assert_eq!(file_name("C:\\a\\B.exe"), "B.exe");
    }
}
