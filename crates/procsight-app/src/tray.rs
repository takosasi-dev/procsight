//! トレイのアイコンとメニュー(FR-47)。設定でオンのときだけ作る。アイコンは 16×16 に CPU 使用率の棒を描いて差し替える。
//! 作るのは画面のスレッド(INV-9 の例外)。メニューは「開く」「終了」、ダブルクリックでも開く。
//! イベントが来たら egui を起こす(更新を一時停止して隠していても、メニューが効くように)。

use crate::icon_art::tray_cpu_rgba;
use eframe::egui;
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
    /// 前に描いた棒の高さ(同じなら差し替えない)
    last: Option<u8>,
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
        Some(Tray { icon: tray, events: rx, last: None })
    }

    pub fn set_cpu(&mut self, pct: Option<f32>) {
        let p = pct.unwrap_or(0.0);
        let step = (p.clamp(0.0, 100.0) / 100.0 * 14.0).round() as u8;
        if self.last == Some(step) {
            return;
        }
        self.last = Some(step);
        if let Ok(icon) = Icon::from_rgba(tray_cpu_rgba(p), 16, 16) {
            let _ = self.icon.set_icon(Some(icon));
        }
        let _ = self.icon.set_tooltip(Some(format!("ProcSight  CPU {}", procsight_core::format::pct(pct))));
    }

    /// 溜まったメニューとアイコンのイベント
    pub fn poll(&self) -> Vec<TrayEvent> {
        self.events.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::icon_art::tray_cpu_rgba;

    fn filled_rows(rgba: &[u8]) -> usize {
        // 真ん中の列(x=8)で、棒の色の行を数える
        (1..15).filter(|y| rgba[(y * 16 + 8) * 4] != 0x1E).count()
    }

    #[test]
    fn bar_grows_with_cpu() {
        assert_eq!(tray_cpu_rgba(0.0).len(), 16 * 16 * 4);
        assert_eq!(filled_rows(&tray_cpu_rgba(0.0)), 1);
        assert_eq!(filled_rows(&tray_cpu_rgba(50.0)), 7);
        assert_eq!(filled_rows(&tray_cpu_rgba(100.0)), 14);
        assert_eq!(filled_rows(&tray_cpu_rgba(f32::NAN)), 1);
    }
}
