//! `procsight-probe ifeo`(管理者で利用者が手で動かす。§9.5)と `ifeo-echo`(置き換えから起動されたときの記録)。
//! Debugger の値の読み書きは担当 B の taskmgr_replace の関数だけで行う(INV-13)。途中で失敗しても、終わる前に必ず値を (1) に戻す。

use crate::{find, try_b};
use procsight_win::nt::spi::pids_named;
use procsight_win::{ProcessGate, static_info, taskmgr_replace};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn echo_file() -> PathBuf {
    std::env::temp_dir().join("procsight-ifeo-echo.txt")
}

/// パスの「\Users\<名前>\」の名前を伏せる
fn mask(s: &str) -> String {
    let lower = s.to_lowercase();
    let Some(i) = lower.find("\\users\\") else { return s.to_string() };
    let start = i + "\\users\\".len();
    let end = s[start..].find('\\').map_or(s.len(), |j| start + j);
    format!("{}<ユーザー>{}", &s[..start], &s[end..])
}

fn wait_echo(prompt: &str) {
    let _ = std::fs::remove_file(echo_file());
    println!("  {prompt}(60 秒待ちます)");
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(t) = std::fs::read_to_string(echo_file()) {
            println!("  {} に書かれた内容:", mask(&echo_file().to_string_lossy()));
            for line in t.lines() {
                println!("    {}", mask(line));
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    println!("  60 秒の間に ifeo-echo が起動されませんでした");
}

fn current() -> Option<Option<String>> {
    try_b(taskmgr_replace::read_info).map(|i| i.debugger)
}

pub fn run() {
    println!("[ifeo]");
    if !static_info::self_elevated() {
        println!("  管理者で動かしてください");
        std::process::exit(1);
    }
    let Some(original) = current() else { return };
    println!("  (1) 今の Debugger: {}", original.as_deref().map_or("なし".into(), mask));
    let exe = std::env::current_exe().expect("probe の exe").to_string_lossy().into_owned();
    let echo_value = format!("\"{exe}\" ifeo-echo");
    let steps = catch_unwind(AssertUnwindSafe(|| {
        // 試しの値をそのまま書く(Phase 0 の試し。INV-15 の例外として §9.5 が認める)
        if let Err(e) = taskmgr_replace::probe_set_raw(Some(&echo_value)) {
            println!("  (2) 書けません: {}", e.message(true));
            return;
        }
        println!("  (2) Debugger に {} を書きました", mask(&echo_value));
        wait_echo("Ctrl+Shift+Esc を押してください");
        println!("  (3)");
        wait_echo("Ctrl+Alt+Del の画面で「タスク マネージャー」を選んでください");
        let gate = ProcessGate::new(std::process::id(), true);
        for i in 1..=5 {
            let (r, restore_failed) = taskmgr_replace::probe_open_task_manager(&gate);
            std::thread::sleep(Duration::from_secs(2));
            let opened = !pids_named("Taskmgr.exe").is_empty();
            let back = current().flatten().as_deref() == Some(echo_value.as_str());
            println!(
                "  (4) {i} 回目: 結果 {}、開いた {}、値が戻った {}(戻せなかった印 {})",
                r.map_or_else(|e| e.message(true), |_| "成功".into()),
                if opened { "はい" } else { "いいえ" },
                if back { "はい" } else { "いいえ" },
                if restore_failed.is_some() { "あり" } else { "なし" }
            );
            println!("      タスク マネージャーを閉じてください(60 秒待ちます)");
            let deadline = Instant::now() + Duration::from_secs(60);
            while !pids_named("Taskmgr.exe").is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }));
    // (5) 必ず戻す
    // (1) に無ければ消し、あればその値をそのまま書く
    let r = try_b(|| taskmgr_replace::probe_set_raw(original.as_deref()));
    let now = current().flatten();
    println!(
        "  (5) 値を戻しました: {}(今の値 {}、(1) と同じ: {})",
        match r {
            Some(Ok(())) => "成功".to_string(),
            Some(Err(e)) => e.message(true),
            None => "呼べません".into(),
        },
        now.as_deref().map_or("なし".into(), mask),
        if now == original { "はい" } else { "いいえ" }
    );
    if steps.is_err() {
        println!("  途中で失敗しました(値は戻してあります)");
    }
}

/// 置き換えから起動されたとき: 引数・昇格・親の名前を %TEMP%\procsight-ifeo-echo.txt に書いて終わる
pub fn echo(args: &[String]) {
    let parent = find(std::process::id())
        .and_then(|me| find(me.parent_pid))
        .map_or("取れない".to_string(), |p| p.image_name);
    let text = format!(
        "引数: {:?}\n昇格: {}\n親: {}\n",
        args,
        if static_info::self_elevated() { "はい" } else { "いいえ" },
        parent
    );
    let _ = std::fs::write(echo_file(), text);
}
