//! waypoint — ホットキーで呼び出す階層ポップアップランチャー。
//!
//! 現時点ではトレイに常駐して終了できるところまで。
//! ポップアップメニュー・フック・設定読み込みは未実装。

// コンソールウィンドウを出さない
#![windows_subsystem = "windows"]

mod tray;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() {
    // メニューの座標は物理ピクセルで扱うため、モニタごとの DPI を
    // 自前で解釈する。ウィンドウを作る前に宣言する必要がある。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // GUI サブシステムのためコンソールに出力できない。
    // 常駐せずに結果をファイルへ書いて終了する自己診断モード。
    let selftest = std::env::args().any(|a| a == "--selftest");

    let hwnd = match tray::install() {
        Ok(hwnd) => hwnd,
        Err(err) => {
            if selftest {
                write_selftest_result(&format!("FAIL: tray install: {err}"));
            }
            return;
        }
    };

    if selftest {
        write_selftest_result(&format!(
            "PASS: message window={:?}, tray icon added",
            hwnd.0
        ));
        tray::remove(hwnd);
        return;
    }

    run_message_loop();

    tray::remove(hwnd);
}

/// 自己診断の結果を %TEMP%\waypoint_selftest.txt に書く。
fn write_selftest_result(text: &str) {
    if let Ok(dir) = std::env::var("TEMP") {
        let _ = std::fs::write(format!("{dir}\\waypoint_selftest.txt"), text);
    }
}

fn run_message_loop() {
    let mut msg = MSG::default();
    unsafe {
        // GetMessageW は終了時に 0、エラー時に -1 を返す
        while GetMessageW(&mut msg, Some(HWND::default()), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
