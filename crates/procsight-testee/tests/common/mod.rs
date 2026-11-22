//! 結合テストの共通: 自分が起動した testee(Drop で必ず終わらせる)と、待つための小道具。

#![allow(dead_code)]

use procsight_core::{ProcId, RawProcess};
use procsight_win::nt::spi::{Fields, SpiReader};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub struct Testee {
    pub child: Child,
    pub id: ProcId,
}

impl Testee {
    /// --exit-after 60 を付ける(テストが落ちても残らない保険。一時停止中は効かないので Drop で終わらせる)
    pub fn start(args: &[&str]) -> Testee {
        let child = Command::new(env!("CARGO_BIN_EXE_procsight-testee"))
            .args(args)
            .args(["--exit-after", "60"])
            .spawn()
            .expect("testee を起動");
        let mut t = Testee { id: ProcId { pid: child.id(), create_time: 0 }, child };
        let p = wait_for(Duration::from_secs(5), || find(t.id.pid)).expect("testee が SPI に現れる");
        t.id = p.id;
        t
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Testee {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn find(pid: u32) -> Option<RawProcess> {
    SpiReader::new().read(Fields::ALL).ok()?.into_iter().find(|p| p.id.pid == pid)
}

pub fn cpu_time(pid: u32) -> u64 {
    let p = find(pid).expect("testee がいる");
    p.kernel_time.unwrap() + p.user_time.unwrap()
}

/// f が Some を返すまで 50 ミリ秒ごとに試す
pub fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
