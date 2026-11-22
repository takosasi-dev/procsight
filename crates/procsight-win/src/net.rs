//! ネットワークのアダプターごとの送受信の速さ(担当 A)。GetIfTable2 の差。ループバックと Up でない物は出さない(D-13)。
//! LWF などのフィルターの仮のインターフェースも出さない(同じ量を数えた行が増えるため)。

use procsight_core::NetSample;
use std::collections::HashMap;
use std::time::Instant;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

/// MIB_IF_ROW2 の InterfaceAndOperStatusFlags の FilterInterface(2 ビット目)
const FILTER_INTERFACE: u8 = 0b10;

struct Row {
    luid: u64,
    alias: String,
    up: bool,
    shown: bool,
    rx: u64,
    tx: u64,
}

fn shown(r: &MIB_IF_ROW2) -> bool {
    r.Type != IF_TYPE_SOFTWARE_LOOPBACK
        && r.OperStatus == IfOperStatusUp
        && r.InterfaceAndOperStatusFlags._bitfield & FILTER_INTERFACE == 0
}

fn rows() -> Option<Vec<Row>> {
    let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: t はこの関数の変数。成功したら下で FreeMibTable で 1 回だけ解放する
    if unsafe { GetIfTable2(&mut t) }.0 != 0 || t.is_null() {
        return None;
    }
    // SAFETY: t は GetIfTable2 が返した表で、Table には NumEntries 個の行が続く
    let rows = unsafe { std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize) };
    let out = rows
        .iter()
        .map(|r| Row {
            // SAFETY: NET_LUID_LH は u64 の union で、Value はどの形でも読める
            luid: unsafe { r.InterfaceLuid.Value },
            alias: crate::util::from_wide(&r.Alias),
            up: r.OperStatus == IfOperStatusUp,
            shown: shown(r),
            rx: r.InOctets,
            tx: r.OutOctets,
        })
        .collect();
    // SAFETY: t は GetIfTable2 が返した表で、ここで 1 回だけ解放する。rows はこの後使わない
    unsafe { FreeMibTable(t.cast()) };
    Some(out)
}

#[derive(Default)]
pub struct NetSampler {
    prev: HashMap<u64, (u64, u64)>,
    prev_at: Option<Instant>,
}

impl NetSampler {
    /// 1 回目は前回が無いので None。表が取れなければ None(E-11)
    pub fn sample(&mut self) -> Option<Vec<NetSample>> {
        let rows = rows()?;
        let now = Instant::now();
        let elapsed = self.prev_at.map(|t| now.duration_since(t).as_secs_f64());
        let mut out = Vec::new();
        let mut next = HashMap::new();
        for r in rows.into_iter().filter(|r| r.shown) {
            if let (Some(secs), Some((rx, tx))) = (elapsed, self.prev.get(&r.luid))
                && secs > 0.0
            {
                out.push(NetSample {
                    name: r.alias.clone(),
                    recv_bps: r.rx.saturating_sub(*rx) as f64 / secs,
                    send_bps: r.tx.saturating_sub(*tx) as f64 / secs,
                });
            }
            next.insert(r.luid, (r.rx, r.tx));
        }
        let first = self.prev_at.is_none();
        self.prev = next;
        self.prev_at = Some(now);
        if first { None } else { Some(out) }
    }
}

/// probe の [net]: (アダプターの数, Up の数, 出す数)
pub fn counts() -> Option<(usize, usize, usize)> {
    let rows = rows()?;
    Some((rows.len(), rows.iter().filter(|r| r.up).count(), rows.iter().filter(|r| r.shown).count()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn table_reads() {
        let (all, up, shown) = super::counts().unwrap();
        assert!(all >= up && up >= shown && all > 0);
        let mut s = super::NetSampler::default();
        assert!(s.sample().is_none());
        assert!(s.sample().is_some());
    }
}
