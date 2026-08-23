//! waypoint のライブラリ部分。
//!
//! 純ロジック (設定の解析・変数展開・メニュー構築) をここに置き、
//! 統合テストから叩けるようにする。Win32 に触る部分は
//! バイナリ側 (main.rs) が持つ。

pub mod apps;
pub mod autostart;
pub mod bookmarks;
pub mod browser_history;
pub mod clipboard;
pub mod config;
pub mod dynamic;
pub mod everything;
pub mod favicons;
pub mod folder_import;
pub mod git;
pub mod hotkey_capture;
pub mod icon;
pub mod known_folder;
pub mod menu;
pub mod menu_draw;
pub mod panic_log;
pub mod process;
pub mod quick_launch;
pub mod quick_launch_history;
pub mod quick_launch_window;
pub mod romaji;
pub mod shell;
pub mod single;
pub mod theme;
pub mod tray;
pub mod trigger;
