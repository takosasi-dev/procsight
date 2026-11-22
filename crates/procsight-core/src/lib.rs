//! ProcSight の Windows に依存しない部分。取得した生の値から画面の行を作り、並べ替え・絞り込み・
//! 操作を断る判定・設定の読み書きを行う。Windows の crate を参照せず、unsafe も使わない(仕様書 D-2)。
//! Linux でも `cargo test -p procsight-core` が通る(NFR-15)。
#![forbid(unsafe_code)]

pub mod config;
pub mod diff;
pub mod filter;
pub mod format;
pub mod model;
pub mod ring;
pub mod rules;
pub mod startup;
pub mod taskmgr;
pub mod tree;

pub use model::*;
