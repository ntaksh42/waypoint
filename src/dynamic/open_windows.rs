//! 開いているウィンドウの列挙 (Quick Launch の `w ` プレフィックスとトレイの
//! "Current Windows" の元)。`dynamic::refresh_async` が Recent の走査とは別
//! スレッドで呼ぶ。

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::WindowEntry;

/// タスクバー / Alt+Tab に出る「実際に開いているウィンドウ」かどうか。
///
/// `EnumWindows` は可視ならヘルパーウィンドウ (`Program Manager` など)
/// まで返してしまう。オーナーを持つウィンドウとツールウィンドウを除く
/// (`WS_EX_APPWINDOW` が立っていれば強制的に含める) のが Explorer と
/// 同じ判定基準。
///
/// UWP / ApplicationFrameHost 系アプリ (設定アプリなど) は、可視状態
/// のままホスト側のウィンドウを DWM 上で覆い隠す (`DWMWA_CLOAKED`) 。
/// これを見ないと同じアプリが 2 枚重複して列挙される (実測で確認済み) 。
fn is_taskbar_window(hwnd: HWND) -> bool {
    unsafe {
        if is_cloaked(hwnd) {
            return false;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_APPWINDOW.0 != 0 {
            return true;
        }
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        GetWindow(hwnd, GW_OWNER).is_err()
    }
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        let result = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            std::ptr::from_mut(&mut cloaked).cast(),
            size_of::<u32>() as u32,
        );
        result.is_ok() && cloaked != 0
    }
}

pub(super) fn enumerate_windows() -> Vec<WindowEntry> {
    let mut windows = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_window),
            LPARAM((&mut windows as *mut Vec<WindowEntry>) as isize),
        );
    }
    windows
}

/// タスクバーに出るウィンドウを持つプロセスの PID と、その先頭ウィンドウのタイトル。
/// `k ` でタスクマネージャーの「アプリ」相当を上に出すために使う。
pub(crate) fn window_titles_by_pid() -> std::collections::HashMap<u32, String> {
    let mut titles = std::collections::HashMap::new();
    for window in enumerate_windows() {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(HWND(window.hwnd as *mut c_void), Some(&mut pid)) };
        titles.entry(pid).or_insert(window.title);
    }
    titles
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == GetCurrentProcessId() {
            return true.into();
        }
        if !is_taskbar_window(hwnd) {
            return true.into();
        }
        // GetWindowTextLengthW を判定用と確保用の 2 回に分けて呼ぶと、
        // その間の別ウィンドウへの Win32 呼び出しが挟まる間にタイトルが
        // 変わり得て (通知件数の更新など) 、1 回目は空でないのに 2 回目が
        // 0 を返してウィンドウごと取りこぼす余地があった。1 回だけ呼んで
        // その結果でバッファを確保し、空なら以降は評価しない
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return true.into();
        }
        let mut title = vec![0u16; length as usize + 1];
        let len = GetWindowTextW(hwnd, &mut title);
        if len > 0 {
            let windows = &mut *(lparam.0 as *mut c_void as *mut Vec<WindowEntry>);
            windows.push(WindowEntry {
                title: String::from_utf16_lossy(&title[..len as usize]),
                hwnd: hwnd.0 as isize,
                process_name: crate::process::process_name_of(hwnd).unwrap_or_default(),
            });
        }
        true.into()
    }
}
