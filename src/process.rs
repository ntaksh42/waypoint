//! 最前面ウィンドウの実行ファイル名を調べる。
//!
//! 中ボタンを横取りしてよいかの判定に使う。ブラウザのオートスクロールを
//! 壊さないため、除外リストにあるプロセスでは何もしない (FR-1.4 / R-1) 。

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

/// 最前面ウィンドウの実行ファイル名 (例: `chrome.exe`) 。
pub fn foreground_process_name() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    process_name_of(hwnd)
}

/// 指定ウィンドウを所有するプロセスの実行ファイル名。
pub fn process_name_of(hwnd: HWND) -> Option<String> {
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }

    unsafe {
        // 名前を読むだけなので最小の権限で開く。
        // これなら昇格プロセスでも成功することがある。
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;

        let full = String::from_utf16_lossy(&buf[..len as usize]);
        // フルパスから実行ファイル名だけ取り出す
        Some(full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string())
    }
}

/// 除外リストに含まれるか。大文字小文字は無視する。
pub fn is_excluded(name: &str, excluded: &[String]) -> bool {
    excluded.iter().any(|e| e.eq_ignore_ascii_case(name))
}
