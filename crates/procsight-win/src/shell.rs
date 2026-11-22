//! エクスプローラーとシェルの呼び出し(担当 B。FR-16・FR-23・FR-35)。場所を開く・プロパティ・タスク マネージャー・
//! ファイルを選ぶ窓。どれも窓を出すのでテストでは動かさず、渡す文字列だけを確かめる。COM の初期化の補助もここに置く。

use crate::jobs::OpError;
use crate::util::{pcwstr, wide, win32_code};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};

/// 呼んだスレッドで COM を初期化し、Drop で戻す。すでに別の形で初期化されていればそのまま使う(戻さない)
pub(crate) struct Com(bool);

impl Com {
    pub(crate) fn init() -> Com {
        // SAFETY: 引数の値だけを渡す。S_OK と S_FALSE のときだけ Drop で CoUninitialize を呼ぶ
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        Com(hr.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: 同じスレッドで成功した CoInitializeEx と対にする
            unsafe { CoUninitialize() };
        }
    }
}

fn system32(exe: &str) -> String {
    format!("{}\\System32\\{exe}", crate::util::system_root())
}

fn shell_execute(file: &str, params: Option<&str>) -> Result<(), OpError> {
    let _com = Com::init();
    let f = wide(file);
    let p = params.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpFile: pcwstr(&f),
        lpParameters: p.as_ref().map(|p| pcwstr(p)).unwrap_or_default(),
        nShow: 1, // SW_SHOWNORMAL
        ..Default::default()
    };
    // SAFETY: info の文字列はこの関数の中の配列を指し、呼び出しの間生きている
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| OpError::from_code(win32_code(&e)))
}

/// explorer.exe に渡す引数
fn select_args(path: &str) -> String {
    format!("/select,\"{}\"", path.trim_matches('"'))
}

/// `explorer.exe /select,"<パス>"` を ShellExecuteExW で起動する
pub fn open_location(path: &str) -> Result<(), OpError> {
    shell_execute(&format!("{}\\explorer.exe", crate::util::system_root()), Some(&select_args(path)))
}

/// SHObjectProperties(SHOP_FILEPATH)
pub fn show_properties(path: &str) -> Result<(), OpError> {
    use windows::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties};
    let _com = Com::init();
    let p = wide(path);
    // SAFETY: p は 0 で終わる配列で、呼び出しの間生きている
    let ok = unsafe { SHObjectProperties(None, SHOP_FILEPATH, pcwstr(&p), windows::core::PCWSTR::null()) };
    // SAFETY: 引数なし。直前の呼び出しのエラー番号を読むだけ
    if ok.as_bool() { Ok(()) } else { Err(OpError::from_code(unsafe { windows::Win32::Foundation::GetLastError() }.0)) }
}

/// taskmgr.exe を引数なしで起動する(置き換えがオフのとき)
pub fn launch_taskmgr() -> Result<(), OpError> {
    shell_execute(&system32("taskmgr.exe"), None)
}

/// IFileOpenDialog(複数選択)。やめたら空。COM を STA で使うため専用のスレッドで出す
pub fn pick_files() -> Vec<String> {
    std::thread::spawn(pick_files_sta).join().unwrap_or_default()
}

fn pick_files_sta() -> Vec<String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    let _com = Com::init();
    // SAFETY: COM はこのスレッドで初期化した。返った文字列は CoTaskMemFree で返す
    unsafe {
        let Ok(dlg) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else {
            return Vec::new();
        };
        let Ok(opts) = dlg.GetOptions() else { return Vec::new() };
        if dlg.SetOptions(opts | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM).is_err()
            || dlg.Show(None).is_err()
        {
            return Vec::new(); // やめた(ERROR_CANCELLED)も含む
        }
        let Ok(items) = dlg.GetResults() else { return Vec::new() };
        let n = items.GetCount().unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            if let Ok(item) = items.GetItemAt(i)
                && let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH)
            {
                out.push(crate::util::pwstr_to_string(p));
                CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_without_launching() {
        assert_eq!(select_args("C:\\a b\\c.exe"), "/select,\"C:\\a b\\c.exe\"");
        assert_eq!(select_args("\"C:\\x.exe\""), "/select,\"C:\\x.exe\"");
        assert!(system32("taskmgr.exe").to_lowercase().ends_with("\\system32\\taskmgr.exe"));
        let _c = Com::init();
        let _d = Com::init(); // 二重でも落ちない
    }
}
