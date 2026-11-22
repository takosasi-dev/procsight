//! ProcSight の画面(eframe / egui)。Win32 を直接呼ばず、unsafe も書かない(D-2・INV-8・INV-9)。
//! 画面の状態(app::State)と Backend を持つ殻(shell)を分け、Backend 無しでも demo の値で全部の画面を描ける。
//! bin(main.rs)は shell::run を呼ぶだけ。
#![forbid(unsafe_code)]

pub mod about;
pub mod app;
pub mod demo;
pub mod details;
pub mod dialogs;
pub mod export;
pub mod files;
pub mod fonts;
pub mod icon_art;
pub mod locks;
pub mod log;
pub mod mini;
pub mod palette;
pub mod perf;
pub mod procs;
pub mod services_view;
pub mod settings;
pub mod shell;
pub mod startup_view;
pub mod theme;
pub mod tray;
pub mod ttc;
pub mod widgets;
