//! waypoint — ホットキーで呼び出す階層ポップアップランチャー。

// コンソールウィンドウを出さない
#![windows_subsystem = "windows"]

use waypoint::{autostart, shell, single, tray, trigger};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() {
    let selftest = std::env::args().any(|a| a == "--selftest");
    // メニューの座標は物理ピクセルで扱うため、モニタごとの DPI を
    // 自前で解釈する。ウィンドウを作る前に宣言する必要がある。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // 二重起動なら既存プロセスにメニューを出させて終わる (FR-8.3)
    let _instance = match single::acquire() {
        Ok(guard) => guard,
        Err(single::AlreadyRunning) => {
            if !selftest {
                single::signal_existing();
            }
            return;
        }
    };

    // IShellWindows は STA を要求する (R-8)
    let _com = shell::ComGuard::new();

    tray::load_state();

    let hwnd = match tray::install() {
        Ok(hwnd) => hwnd,
        Err(err) => {
            if selftest {
                write_selftest_result(&format!("FAIL: tray install: {err}"));
            }
            return;
        }
    };

    // 設定は tray::load_state() で読み込み済み。ここで再読み込みしない
    let hotkey_ok = tray::register_hotkey_from_config(hwnd);
    tray::set_hotkey_failed(!hotkey_ok);
    let hook = trigger::install_mouse_hook(hwnd).ok();

    if selftest {
        write_selftest_result(&selftest_report(hwnd, hotkey_ok, hook.is_some()));
        cleanup(hwnd, hook);
        return;
    }

    run_message_loop();

    cleanup(hwnd, hook);
}

fn cleanup(hwnd: HWND, hook: Option<windows::Win32::UI::WindowsAndMessaging::HHOOK>) {
    if let Some(h) = hook {
        trigger::remove_mouse_hook(h);
    }
    trigger::unregister_hotkey(hwnd);
    tray::remove(hwnd);
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

/// GUI サブシステムのためコンソールに出力できない。
/// 常駐せずに結果をファイルへ書いて終了する自己診断モード。
fn selftest_report(hwnd: HWND, hotkey_ok: bool, hook_ok: bool) -> String {
    let items = tray::item_count();
    let actions = tray::action_count();
    let spec = tray::hotkey_spec();
    // メニューが組み立てられ、トリガーが両方張れていれば PASS
    let all_ok = hotkey_ok && hook_ok && actions > 0;
    format!(
        "{}: window={:?} items={} actions={} hotkey=\"{}\":{} mouse_hook={} autostart={}",
        if all_ok { "PASS" } else { "FAIL" },
        hwnd.0,
        items,
        actions,
        spec,
        hotkey_ok,
        hook_ok,
        autostart::is_enabled(),
    )
}

fn write_selftest_result(text: &str) {
    if let Ok(dir) = std::env::var("TEMP") {
        let _ = std::fs::write(format!("{dir}\\waypoint_selftest.txt"), text);
    }
}
