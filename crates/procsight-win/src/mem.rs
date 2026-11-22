//! メモリの全体の値(担当 A)。GlobalMemoryStatusEx と GetPerformanceInfo(FR-25・D-13)。

use procsight_core::MemSample;
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

/// 使用中 = 合計 − 利用可能。コミット済みと上限はページ数 × ページの大きさ。取れなければ None
pub fn sample() -> Option<MemSample> {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: m はこの関数の変数で、dwLength に大きさを入れてある
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    let mut p = PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    // SAFETY: p はこの関数の変数で、大きさを渡す
    unsafe { GetPerformanceInfo(&mut p, p.cb) }.ok()?;
    let page = p.PageSize as u64;
    Some(MemSample {
        total: m.ullTotalPhys,
        available: m.ullAvailPhys,
        commit_total: p.CommitTotal as u64 * page,
        commit_limit: p.CommitLimit as u64 * page,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_values() {
        let m = super::sample().unwrap();
        assert!(m.total > m.available && m.available > 0);
        assert!(m.commit_limit >= m.commit_total && m.commit_total > 0);
    }
}
