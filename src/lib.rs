//! waypoint のライブラリ部分。
//!
//! 純ロジック (設定の解析・変数展開・メニュー構築) をここに置き、
//! 統合テストから叩けるようにする。Win32 に触る部分は
//! バイナリ側 (main.rs) が持つ。

pub mod autostart;
pub mod config;
pub mod dynamic;
pub mod folder_import;
pub mod icon;
pub mod known_folder;
pub mod menu;
pub mod process;
pub mod shell;
pub mod single;
pub mod theme;
pub mod tray;
pub mod trigger;
