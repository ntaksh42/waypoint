//! Win32 メッセージ処理本体 (WndProc) 。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, PostQuitMessage, WM_COMMAND, WM_COPYDATA, WM_DESTROY,
    WM_DRAWITEM, WM_HOTKEY, WM_LBUTTONUP, WM_MEASUREITEM, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_THEMECHANGED,
};

use crate::quick_launch;
use crate::quick_launch_history;
use crate::quick_launch_window::{self, WM_QUICK_LAUNCH_ADD_TO_FAVORITES, WM_QUICK_LAUNCH_EXECUTE};
use crate::shell;
use crate::trigger::{self, WM_TRIGGER_MENU};

use super::actions::{
    add_entry_to_favorites, handle_dynamic_refreshed, rebuild_menu, refresh_dynamic, show_launcher,
    show_launcher_at_cursor, show_tray_menu,
};
use super::{
    WM_AZURE_DEVOPS_REFRESHED, WM_DYNAMIC_REFRESHED, WM_RELOAD_CONFIG, WM_TRAY, reload, with_state,
};

/// Win32 から呼ばれる入口。
///
/// `extern "system"` は unwind できず、panic すると abort する。
/// ここが落ちると常駐部ごと消えてトレイアイコンも残らないため、
/// 捕まえてログに残し、既定処理へ流して動作を続ける。
pub(crate) extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match std::panic::catch_unwind(|| dispatch(hwnd, msg, wparam, lparam)) {
        Ok(result) => result,
        Err(_) => {
            crate::panic_log::record(&format!(
                "tray wnd_proc panicked on message 0x{msg:04x}; recovered"
            ));
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lparam.0 as u32) & 0xffff;
            match event {
                // 左クリックでランチャーメニュー (FR-1.3)
                WM_LBUTTONUP => show_launcher_at_cursor(hwnd),
                WM_RBUTTONUP => show_tray_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        // フックまたは二重起動からの要求 (R-4: 構築はここで行う)
        WM_TRIGGER_MENU => {
            let at = POINT {
                x: wparam.0 as i32,
                y: lparam.0 as i32,
            };
            // 座標が無い (二重起動からの通知) 場合はカーソル位置
            if at.x == 0 && at.y == 0 {
                show_launcher_at_cursor(hwnd);
            } else {
                show_launcher(hwnd, at, trigger::origin_window());
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if wparam.0 as i32 == trigger::QUICK_LAUNCH_HOTKEY_ID {
                let origin =
                    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                let _ = quick_launch_window::show(hwnd, Some(origin));
            } else {
                show_launcher_at_cursor(hwnd);
            }
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_EXECUTE => {
            if let Some((entry, origin)) = quick_launch_window::take_pending() {
                quick_launch_history::record(&entry);
                match entry.action {
                    quick_launch::Action::OpenFolder(mode) => {
                        let _ = shell::open(&entry.path, mode, origin);
                    }
                    quick_launch::Action::FocusWindow(hwnd) => {
                        shell::activate_window(HWND(hwnd as *mut _));
                    }
                    quick_launch::Action::FocusBrowserTab(target) => {
                        let _ = crate::browser_tabs::request_focus(&target);
                    }
                    quick_launch::Action::OpenUrl(url) => {
                        let _ = shell::open_shell_item(&url);
                    }
                    quick_launch::Action::OpenWithDefaultHandler
                    | quick_launch::Action::LaunchApp => {
                        let _ = shell::open_shell_item(&entry.path);
                    }
                    // ReplaceQuery / AzureLiveWorkItemSearch / AzureLivePullRequestSearch
                    // はウィンドウを閉じずに Quick Launch 側で完結する
                    // (queue_selected 参照)。ここへは実行対象として来ない
                    quick_launch::Action::ReplaceQuery(_)
                    | quick_launch::Action::AzureLiveWorkItemSearch(_)
                    | quick_launch::Action::AzureLivePullRequestSearch { .. } => {}
                }
                refresh_dynamic(hwnd);
            }
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_ADD_TO_FAVORITES => {
            if let Some(entry) = quick_launch_window::take_pending_add() {
                add_entry_to_favorites(entry);
            }
            LRESULT(0)
        }
        WM_RELOAD_CONFIG => {
            reload(hwnd);
            LRESULT(0)
        }
        WM_AZURE_DEVOPS_REFRESHED => {
            with_state(|state| {
                let state = state.borrow();
                if let Some(state) = state.as_ref() {
                    quick_launch_window::configure(&state.config, &state.dynamic);
                }
            });
            LRESULT(0)
        }
        WM_DYNAMIC_REFRESHED => {
            handle_dynamic_refreshed();
            LRESULT(0)
        }
        WM_COPYDATA if lparam.0 != 0 => {
            unsafe {
                let copy_data =
                    &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                if copy_data.dwData == crate::browser_tabs::SNAPSHOT_COPYDATA
                    && !copy_data.lpData.is_null()
                    && copy_data.cbData > 0
                {
                    // Native Messaging host はこのハンドラから戻ると送信バッファを解放する。
                    // 先に JSON をコピーし、Quick Launch のキャッシュ更新はそのコピーだけで行う。
                    let bytes = std::slice::from_raw_parts(
                        copy_data.lpData.cast::<u8>(),
                        copy_data.cbData as usize,
                    );
                    if let Some((browser, tabs)) = crate::browser_tabs::parse_snapshot(bytes) {
                        quick_launch_window::replace_browser_tabs(browser, tabs);
                    }
                }
            }
            LRESULT(1)
        }
        // ダーク / ライトの切り替えでアイコンの見え方が変わる。
        // 古いビットマップを捨て、次回表示で引き直す
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            crate::icon::clear_cache();
            // メニューフォントとテーマの色も変わる。掴んだまま使わない
            crate::menu_draw::reset_font();
            crate::theme::enable_dark_menus();
            rebuild_menu();
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // オーナードロー項目の採寸と描画 (FR-2.3)
        WM_MEASUREITEM => {
            if crate::menu_draw::measure(wparam, lparam) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_DRAWITEM => {
            if crate::menu_draw::draw(wparam, lparam) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_COMMAND => LRESULT(0),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(crate) fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}
