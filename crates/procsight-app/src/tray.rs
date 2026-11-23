//! トレイのアイコンとメニュー(FR-47)。設定でオンのときだけ作る。アイコンは 16×16 に CPU・メモリの使用率の棒を描いて差し替える。
//! 作るのは画面のスレッド(INV-9 の例外)。メニューは「開く」「終了」、ダブルクリックでも開く。
//! イベントが来たら egui を起こす(更新を一時停止して隠していても、メニューが効くように)。

use crate::icon_art::{tray_both_rgba, tray_cpu_rgba, tray_mem_rgba};
use eframe::egui;
use procsight_core::config::TrayValue;
use procsight_core::format::pct;
use std::sync::mpsc::{Receiver, channel};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    Exit,
}

pub struct Tray {
    icon: TrayIcon,
    events: Receiver<TrayEvent>,
    /// 前に描いた (値, CPU の棒の高さ, メモリの棒の高さ)(同じなら差し替えない)
    last: Option<(TrayValue, u8, u8)>,
    /// 前に入れた説明の文(棒の段が同じでも、数字が変われば入れ直す)
    tip: String,
}

impl Drop for Tray {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}

impl Tray {
    pub fn new(ctx: &egui::Context) -> Option<Tray> {
        let open = MenuItem::new("開く", true, None);
        let exit = MenuItem::new("終了", true, None);
        let menu = Menu::new();
        menu.append(&open).ok()?;
        menu.append(&exit).ok()?;
        let icon = Icon::from_rgba(tray_cpu_rgba(0.0), 16, 16).ok()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("ProcSight")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        let (tx, rx) = channel();
        let (open_id, exit_id) = (open.id().clone(), exit.id().clone());
        let (tx2, c1, c2) = (tx.clone(), ctx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let ev = if e.id == open_id {
                TrayEvent::Open
            } else if e.id == exit_id {
                TrayEvent::Exit
            } else {
                return;
            };
            let _ = tx.send(ev);
            c1.request_repaint();
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if matches!(e, TrayIconEvent::DoubleClick { .. }) {
                let _ = tx2.send(TrayEvent::Open);
                c2.request_repaint();
            }
        }));
        Some(Tray { icon: tray, events: rx, last: None, tip: String::new() })
    }

    /// 設定の値(`tray_value`)と今の CPU・メモリの使用率で描く。毎回呼んでよい(棒の高さと値が同じなら何もしない)
    pub fn set(&mut self, value: TrayValue, cpu: Option<f32>, mem: Option<f32>) {
        let k = key(value, cpu, mem);
        if self.last != Some(k) {
            self.last = Some(k);
            if let Ok(icon) = Icon::from_rgba(icon_rgba(value, cpu, mem), 16, 16) {
                let _ = self.icon.set_icon(Some(icon));
            }
        }
        let tip = tooltip(value, cpu, mem);
        if tip != self.tip {
            let _ = self.icon.set_tooltip(Some(tip.as_str()));
            self.tip = tip;
        }
    }

    /// 溜まったメニューとアイコンのイベント
    pub fn poll(&self) -> Vec<TrayEvent> {
        self.events.try_iter().collect()
    }
}

/// 棒の高さ(0〜14 の段)。取れない値は 0
fn step(p: Option<f32>) -> u8 {
    (p.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0 * 14.0).round() as u8
}

/// 描き直すかを決める印。出さない方の値は見ない(変わっても描き直さない)
pub fn key(value: TrayValue, cpu: Option<f32>, mem: Option<f32>) -> (TrayValue, u8, u8) {
    match value {
        TrayValue::Cpu => (value, step(cpu), 0),
        TrayValue::Mem => (value, 0, step(mem)),
        TrayValue::Both => (value, step(cpu), step(mem)),
    }
}

/// アイコンの絵(16×16 の RGBA)
pub fn icon_rgba(value: TrayValue, cpu: Option<f32>, mem: Option<f32>) -> Vec<u8> {
    let (c, m) = (cpu.unwrap_or(0.0), mem.unwrap_or(0.0));
    match value {
        TrayValue::Cpu => tray_cpu_rgba(c),
        TrayValue::Mem => tray_mem_rgba(m),
        TrayValue::Both => tray_both_rgba(c, m),
    }
}

/// アイコンの説明の文。Cpu は v0.3.0 と同じ文
pub fn tooltip(value: TrayValue, cpu: Option<f32>, mem: Option<f32>) -> String {
    match value {
        TrayValue::Cpu => format!("ProcSight  CPU {}", pct(cpu)),
        TrayValue::Mem => format!("ProcSight  メモリ {}", pct(mem)),
        TrayValue::Both => format!("ProcSight  CPU {}  メモリ {}", pct(cpu), pct(mem)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_art::tray_cpu_rgba;

    const BG: u8 = 0x1E;

    /// 列 x で、棒の色の行を数える
    fn filled_rows(rgba: &[u8], x: usize) -> usize {
        (1..15).filter(|y| rgba[(y * 16 + x) * 4] != BG).count()
    }

    #[test]
    fn bar_grows_with_cpu() {
        assert_eq!(tray_cpu_rgba(0.0).len(), 16 * 16 * 4);
        assert_eq!(filled_rows(&tray_cpu_rgba(0.0), 8), 1);
        assert_eq!(filled_rows(&tray_cpu_rgba(50.0), 8), 7);
        assert_eq!(filled_rows(&tray_cpu_rgba(100.0), 8), 14);
        assert_eq!(filled_rows(&tray_cpu_rgba(f32::NAN), 8), 1);
    }

    #[test]
    fn mem_meter_and_both_bars() {
        let m = icon_rgba(TrayValue::Mem, Some(10.0), Some(50.0));
        assert_eq!(filled_rows(&m, 8), 7, "メモリの値で伸びる");
        assert_ne!(m, tray_cpu_rgba(50.0), "CPU と色で見分けられる");
        let b = icon_rgba(TrayValue::Both, Some(100.0), Some(50.0));
        assert_eq!((filled_rows(&b, 4), filled_rows(&b, 11)), (14, 7), "左が CPU・右がメモリ");
        assert_eq!(filled_rows(&b, 8), 0, "2 本の間は空ける");
        assert_eq!(icon_rgba(TrayValue::Cpu, Some(50.0), Some(90.0)), tray_cpu_rgba(50.0), "Cpu は今の絵のまま");
        assert_eq!(icon_rgba(TrayValue::Both, None, None).len(), 16 * 16 * 4);
    }

    #[test]
    fn tooltip_follows_value() {
        assert_eq!(tooltip(TrayValue::Cpu, Some(12.0), Some(40.0)), "ProcSight  CPU 12.0%");
        assert_eq!(tooltip(TrayValue::Mem, Some(12.0), Some(40.0)), "ProcSight  メモリ 40.0%");
        assert_eq!(tooltip(TrayValue::Both, Some(12.0), None), "ProcSight  CPU 12.0%  メモリ —");
    }

    #[test]
    fn key_ignores_hidden_value_and_tracks_setting() {
        assert_eq!(key(TrayValue::Cpu, Some(50.0), Some(10.0)), key(TrayValue::Cpu, Some(50.0), Some(90.0)));
        assert_ne!(key(TrayValue::Mem, Some(50.0), Some(10.0)), key(TrayValue::Mem, Some(50.0), Some(90.0)));
        assert_ne!(key(TrayValue::Cpu, Some(50.0), Some(50.0)), key(TrayValue::Mem, Some(50.0), Some(50.0)), "設定を変えたら描き直す");
        assert_eq!(key(TrayValue::Cpu, None, None), key(TrayValue::Cpu, Some(f32::NAN), None));
    }
}
