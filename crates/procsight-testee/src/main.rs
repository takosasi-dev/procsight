//! テストのためだけのプロセス(仕様書 §9.7)。配布物に入れない。引数で CPU・メモリ・書き込み・子・スレッド・
//! 名前付きイベント・パイプ・ファイルの掴み・窓・応答なしを作り、--exit-after の秒数(既定 600)で終わる。
//! unsafe はイベント・パイプ・窓の Win32 の呼び出しだけ(procsight-win の外だが、配布しないテスト用)。

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Args {
    busy: u32,
    alloc_mb: u64,
    write_rate: u64,
    children: u32,
    depth: u32,
    threads: u32,
    handles: u32,
    pipe: bool,
    hold_file: Option<String>,
    window: bool,
    hang: Option<u64>,
    exit_after: u64,
}

fn parse(raw: &[String]) -> Result<Args, String> {
    let mut a = Args { exit_after: 600, depth: 1, ..Default::default() };
    let mut it = raw.iter();
    while let Some(k) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{k} の値がありません"));
        let num = |s: String| s.parse::<u64>().map_err(|_| format!("{k} の値が数字ではありません"));
        match k.as_str() {
            "--busy" => a.busy = num(val()?)? as u32,
            "--alloc" => a.alloc_mb = num(val()?)?,
            "--write-rate" => a.write_rate = num(val()?)?,
            "--children" => a.children = num(val()?)? as u32,
            "--depth" => a.depth = num(val()?)? as u32,
            "--threads" => a.threads = num(val()?)? as u32,
            "--handles" => a.handles = num(val()?)? as u32,
            "--pipe" => a.pipe = true,
            "--hold-file" => a.hold_file = Some(val()?),
            "--window" => a.window = true,
            "--hang" => {
                a.hang = Some(num(val()?)?);
                a.window = true;
            }
            "--marker" => {
                val()?;
            }
            "--exit-after" => a.exit_after = num(val()?)?,
            other => return Err(format!("知らない引数: {other}")),
        }
    }
    Ok(a)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let exit_after = args.exit_after;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(exit_after));
        std::process::exit(0);
    });

    if args.children > 0 && args.depth > 0 {
        spawn_children(&raw, args.children, args.depth);
    }
    for _ in 0..args.busy {
        std::thread::spawn(|| {
            let mut x = 0u64;
            loop {
                x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        });
    }
    for _ in 0..args.threads {
        std::thread::spawn(|| {
            loop {
                std::thread::park();
            }
        });
    }
    let _memory = alloc(args.alloc_mb);
    if args.write_rate > 0 {
        let rate = args.write_rate;
        std::thread::spawn(move || write_loop(rate));
    }
    let _file = args.hold_file.as_deref().map(hold_file);
    win::create_events(args.handles);
    if args.pipe {
        win::pipe();
    }
    if args.window {
        win::window_loop(args.hang);
    }
    loop {
        std::thread::park();
    }
}

/// 同じ引数で、--depth を 1 つ減らした子を n 個起動する
fn spawn_children(raw: &[String], n: u32, depth: u32) {
    let mut child_args: Vec<String> = Vec::new();
    let mut it = raw.iter();
    while let Some(k) = it.next() {
        if k == "--depth" {
            it.next();
            continue;
        }
        child_args.push(k.clone());
    }
    child_args.push("--depth".into());
    child_args.push((depth - 1).to_string());
    let exe = std::env::current_exe().expect("自分の exe のパス");
    for _ in 0..n {
        if let Err(e) = std::process::Command::new(&exe).args(&child_args).spawn() {
            eprintln!("子を起動できません: {e}");
        }
    }
}

fn alloc(mb: u64) -> Vec<u8> {
    let mut v = vec![0u8; (mb * 1024 * 1024) as usize];
    // vec! の 0 は書き込まないページのことがあるので、全ページに書く
    for i in (0..v.len()).step_by(4096) {
        v[i] = 1;
    }
    std::hint::black_box(&v);
    v
}

/// 一時フォルダのファイルに rate MB/s で書く。ファイルは閉じたら消え(FILE_FLAG_DELETE_ON_CLOSE)、64 MB ごとに先頭へ戻る
fn write_loop(rate: u64) {
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    let path = std::env::temp_dir().join(format!("procsight-testee-{}.bin", std::process::id()));
    let mut f = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("書き込み先を作れません: {e}");
            return;
        }
    };
    let chunk = vec![0x5Au8; 256 * 1024];
    let chunks_per_sec = (rate * 4).max(1);
    let step = Duration::from_secs_f64(1.0 / chunks_per_sec as f64);
    let start = Instant::now();
    let mut written: u64 = 0;
    let mut pos: u64 = 0;
    loop {
        if pos >= 64 * 1024 * 1024 {
            let _ = f.seek(SeekFrom::Start(0));
            pos = 0;
        }
        if f.write_all(&chunk).is_err() {
            return;
        }
        pos += chunk.len() as u64;
        written += 1;
        let due = start + step * written as u32;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

fn hold_file(path: &str) -> Option<std::fs::File> {
    match std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0).open(path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("ファイルを開けません: {e}");
            None
        }
    }
}

mod win {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows::Win32::System::Threading::CreateEventW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
        TranslateMessage, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{HSTRING, PCWSTR, w};

    /// 名前付きイベント Local\ProcSightTest_<連番> を n 個作って持つ(閉じない)
    pub fn create_events(n: u32) {
        for i in 0..n {
            let name = HSTRING::from(format!("Local\\ProcSightTest_{i}"));
            // SAFETY: 名前は呼び出しの間生きている HSTRING。返ったハンドルは閉じずに持ち続ける(テスト用)
            if let Err(e) = unsafe { CreateEventW(None, true, false, &name) } {
                eprintln!("イベントを作れません: {e}");
                return;
            }
        }
    }

    /// 名前付きパイプ \\.\pipe\ProcSightTest_<PID> を作り、クライアントの側で同期の読み取りを待ち続ける
    pub fn pipe() {
        let name = format!("\\\\.\\pipe\\ProcSightTest_{}", std::process::id());
        let wide = HSTRING::from(name.as_str());
        // SAFETY: 名前は呼び出しの間生きている。返ったハンドルは閉じずに持ち続ける(テスト用)
        let server = unsafe { CreateNamedPipeW(&wide, PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None) };
        if server.is_invalid() {
            eprintln!("パイプを作れません");
            return;
        }
        match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
            Ok(mut client) => {
                std::thread::spawn(move || {
                    let mut buf = [0u8; 16];
                    // サーバーは何も書かないので、ここで止まり続ける
                    let _ = client.read(&mut buf);
                    loop {
                        std::thread::park();
                    }
                });
            }
            Err(e) => eprintln!("パイプにつなげません: {e}"),
        }
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        // SAFETY: 受け取った引数をそのまま既定の処理に渡す
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    /// 見える窓を 1 つ出してメッセージを処理する。hang が Some なら、その秒数の後に処理を止める
    pub fn window_loop(hang: Option<u64>) -> ! {
        let class = w!("ProcSightTestee");
        // SAFETY: GetModuleHandleW(None) は自分の exe。WNDCLASSW の文字列は 'static。窓はこのスレッドで作り、このスレッドで処理する
        unsafe {
            let inst = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let title = HSTRING::from(format!("procsight-testee {}", std::process::id()));
            if let Err(e) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                &title,
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                400,
                200,
                None,
                None,
                Some(inst.into()),
                None,
            ) {
                eprintln!("窓を作れません: {e}");
            }
        }
        let stop_at = hang.map(|s| Instant::now() + Duration::from_secs(s));
        let mut msg = MSG::default();
        loop {
            if stop_at.is_some_and(|t| Instant::now() >= t) {
                loop {
                    std::thread::park(); // メッセージを処理しない(応答なし)
                }
            }
            // SAFETY: msg はこの関数の中の構造体。このスレッドの窓のメッセージだけを処理する
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
