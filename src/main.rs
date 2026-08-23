//! waypoint — ホットキーで呼び出す階層ポップアップランチャー。

// コンソールウィンドウを出さない
#![windows_subsystem = "windows"]

use waypoint::{autostart, panic_log, shell, single, theme, tray, trigger};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() {
    // GUI サブシステムでは panic の出力先が無い。最初に登録して
    // 以降の panic をログに残す
    panic_log::install();

    if std::env::args().any(|a| a == "--open-tab-setup-guide") {
        let _ = waypoint::browser_tabs::open_setup_guide();
        return;
    }

    // インストーラーが初回インストール時のみ渡す一度きりのフラグ。
    // 常駐は開始せず、自動起動 (FR-8.4) を有効にして即終了する。
    // トレイの手動トグルと同じ関数を呼ぶことで管理主体をアプリ側に一本化し、
    // MSI がレジストリキーを直接持つことによる二重管理を避ける (installer/waypoint.wxs 参照)。
    if std::env::args().any(|a| a == "--enable-autostart-once") {
        let _ = autostart::set_enabled(true);
        return;
    }

    let selftest = std::env::args().any(|a| a == "--selftest");
    // メニューの座標は物理ピクセルで扱うため、モニタごとの DPI を
    // 自前で解釈する。ウィンドウを作る前に宣言する必要がある。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    theme::enable_dark_menus();

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

    // Native Messaging host はブラウザが起動する。ここでは Chrome / Edge が
    // host を見つけられるよう登録だけを済ませ、手作業を発生させない。
    let _ = waypoint::browser_tabs::register_native_host();

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
    let hotkey = tray::register_hotkey_from_config(hwnd);
    let quick_launch_hotkey = tray::register_quick_launch_hotkey_from_config(hwnd);
    tray::set_hotkey_failed(!hotkey.is_active());
    tray::set_quick_launch_hotkey_failed(!quick_launch_hotkey.is_active());
    let hook = trigger::install_mouse_hook(hwnd).ok();

    if selftest {
        write_selftest_result(&selftest_report(hwnd, hotkey, hook.is_some()));
        cleanup(hwnd, hook);
        return;
    }

    tray::refresh_azure_devops(hwnd);

    run_message_loop();

    cleanup(hwnd, hook);
}

fn cleanup(hwnd: HWND, hook: Option<windows::Win32::UI::WindowsAndMessaging::HHOOK>) {
    if let Some(h) = hook {
        trigger::remove_mouse_hook(h);
    }
    trigger::unregister_hotkeys(hwnd);
    tray::remove(hwnd);
}

fn run_message_loop() {
    let mut msg = MSG::default();
    unsafe {
        // GetMessageW は終了時に 0、エラー時に -1 を返す
        while GetMessageW(&mut msg, Some(HWND::default()), 0, 0).as_bool() {
            if !waypoint::quick_launch_window::handle_message(&msg) {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// GUI サブシステムのためコンソールに出力できない。
/// 常駐せずに結果をファイルへ書いて終了する自己診断モード。
fn selftest_report(hwnd: HWND, hotkey: trigger::Registration, hook_ok: bool) -> String {
    let items = tray::item_count();
    let actions = tray::action_count();
    let spec = tray::hotkey_spec();
    // メニューが組み立てられ、トリガーが両方張れていれば PASS。
    // ホットキーは native (RegisterHotKey) と hook (横取り) のどちらでも可
    let all_ok = hotkey.is_active() && hook_ok && actions > 0;
    let mut out = format!(
        "{}: window={:?} items={} actions={} hotkey=\"{}\":{} mouse_hook={} autostart={}",
        if all_ok { "PASS" } else { "FAIL" },
        hwnd.0,
        items,
        actions,
        spec,
        hotkey.label(),
        hook_ok,
        autostart::is_enabled(),
    );

    // 各項目が実際にどのパスへ解決されたか。
    // 存在しないパスはメニューで選んでも何も起きないので、ここで分かるようにする。
    out.push_str("\n\n-- menu items --\n");
    for (id, mode, path) in tray::dump_actions() {
        let status = if mode == "activateWindow" {
            "WINDOW "
        } else if mode == "openShell" {
            "SHELL  "
        } else if std::path::Path::new(&path).exists() {
            "OK     "
        } else {
            "MISSING"
        };
        out.push_str(&format!("  [{id}] {mode:<9} {} {path}\n", status));
    }
    out
}

fn write_selftest_result(text: &str) {
    if let Ok(dir) = std::env::var("TEMP") {
        let _ = std::fs::write(format!("{dir}\\waypoint_selftest.txt"), text);
    }
}
