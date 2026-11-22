//! 単一インスタンス(担当 B。FR-46・D-24・E-34)。名前付きミューテックス `Local\ProcSight.SingleInstance` と
//! 名前付きイベント `Local\ProcSight.Activate`。イベントは「前面に出す」以外の意味を持たない。
//! イベントを先に作るので、2 つ目は必ずそれを見つけて立てられる。管理者の物を開けなければ E-34。

use crate::util::{pcwstr, wide, win32_code};
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject};

const MUTEX_NAME: &str = "Local\\ProcSight.SingleInstance";
const EVENT_NAME: &str = "Local\\ProcSight.Activate";
const ERROR_ACCESS_DENIED: u32 = 5;

pub enum Acquire {
    First(Instance),
    /// 別の ProcSight にイベントを立てた(終了コード 2 で終える)
    ActivatedOther,
    /// 管理者で動いている ProcSight のイベントを開けなかった(E-34)
    DeniedByElevated,
}

/// 名前付きの同期オブジェクトのハンドル。Drop で閉じる
struct Owned(HANDLE);
// SAFETY: カーネルのハンドルはスレッドに結び付かず、待つ・立てるはどのスレッドからでもよい。閉じるのは Drop の 1 回だけ
unsafe impl Send for Owned {}
// SAFETY: 同上。共有して行うのは WaitForSingleObject だけ
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: CreateMutexW / CreateEventW が返した有効なハンドルで、1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Instance {
    _mutex: Owned,
    event: Arc<Owned>,
}

impl Instance {
    /// イベントを待つスレッドを起こす。立つたびに `on_activate` を呼ぶ
    pub fn listen(&self, on_activate: Box<dyn Fn() + Send + 'static>) {
        let ev = Arc::clone(&self.event);
        let _ = std::thread::Builder::new().name("activate".into()).spawn(move || {
            // SAFETY: ev はこのスレッドが Arc で持ち続けるので、待っている間は閉じられない
            while unsafe { WaitForSingleObject(ev.0, INFINITE) } == WAIT_OBJECT_0 {
                on_activate();
            }
        });
    }
}

/// 作る(あれば開く)。戻り値は (ハンドル, もともとあったか)。失敗したら Win32 のエラー
fn create(name: &str, mutex: bool) -> Result<(Owned, bool), u32> {
    let n = wide(name);
    // SAFETY: n は 0 で終わる。成功の直後に GetLastError で「すでにあった」かを読む
    unsafe {
        let r = if mutex { CreateMutexW(None, false, pcwstr(&n)) } else { CreateEventW(None, false, false, pcwstr(&n)) };
        let h = r.map_err(|e| win32_code(&e))?;
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        Ok((Owned(h), existed))
    }
}

fn acquire_named(mutex_name: &str, event_name: &str) -> Acquire {
    let event = match create(event_name, false) {
        Ok((e, _)) => e,
        Err(ERROR_ACCESS_DENIED) => return Acquire::DeniedByElevated,
        Err(_) => return Acquire::ActivatedOther, // 作れないなら起動を続けない(2 つ目とみなす)
    };
    match create(mutex_name, true) {
        Ok((m, false)) => Acquire::First(Instance { _mutex: m, event: Arc::new(event) }),
        Ok((_, true)) => {
            // SAFETY: event は上で開いた有効なハンドル
            let _ = unsafe { SetEvent(event.0) };
            Acquire::ActivatedOther
        }
        Err(ERROR_ACCESS_DENIED) => Acquire::DeniedByElevated,
        Err(_) => Acquire::ActivatedOther,
    }
}

pub fn acquire() -> Acquire {
    acquire_named(MUTEX_NAME, EVENT_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn second_activates_first() {
        // 本物の名前は取らない
        let id = std::process::id();
        let (m, e) = (format!("Local\\ProcSightTest.SI.{id}"), format!("Local\\ProcSightTest.Act.{id}"));
        let Acquire::First(inst) = acquire_named(&m, &e) else { panic!("1 つ目が First でない") };
        let (tx, rx) = channel();
        inst.listen(Box::new(move || {
            let _ = tx.send(());
        }));
        assert!(matches!(acquire_named(&m, &e), Acquire::ActivatedOther));
        assert!(rx.recv_timeout(Duration::from_secs(3)).is_ok(), "前面に出す知らせが来ない");
        drop(inst);
        // 1 つ目が終われば、次はまた First
        assert!(matches!(acquire_named(&m, &e), Acquire::First(_)));
    }
}
