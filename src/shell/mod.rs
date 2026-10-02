//! フォルダを開く。新規ウィンドウと、既存ウィンドウのフォルダ変更の 2 通り。

use std::path::Path;

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellExecuteW, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
};
use windows::core::{BSTR, HSTRING, Interface, w};

use crate::config::OpenMode;

mod launch;
#[cfg(test)]
mod tests;
pub use launch::{open_claude_code, open_codex, open_editor, open_terminal, resume_agent_session};

/// COM を STA で初期化する。プロセスで一度だけ呼ぶ。
///
/// `IShellWindows` は STA を要求するため、UI スレッドから呼ぶこと (R-8) 。
pub struct ComGuard;

impl ComGuard {
    pub fn new() -> Self {
        unsafe {
            // 既に初期化済みでもエラーにはしない
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Self
    }
}

impl Default for ComGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// 指定パスを開く。
///
/// `reuse` で、かつ `origin` が既存のエクスプローラーウィンドウなら
/// そのウィンドウのフォルダを変更する。該当しなければ新規ウィンドウで開く
/// (FR-4.2 のフォールバック) 。
pub fn open(path: &str, mode: OpenMode, origin: Option<HWND>) -> std::io::Result<()> {
    if !Path::new(path).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("path not found: {path}"),
        ));
    }

    if mode == OpenMode::Reuse
        && let Some(hwnd) = origin
        && navigate_existing(hwnd, path).is_ok()
    {
        return Ok(());
    }

    open_new_window(path)
}

/// Current Windows で選んだウィンドウを復元して前面へ移す。
///
/// 復元は相手の応答を待たず、前面化も入力キューを結合せずに依頼する。
/// `AttachThreadInput` で対象と結合すると、相手が応答しないときに
/// `SetForegroundWindow` まで同期的に待たされ、常駐部全体が固まる。
/// Quick Launch / トレイメニューの選択はユーザー入力に由来するため、
/// 通常の `SetForegroundWindow` の前面化条件を満たす。
pub fn activate_window(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// エクスプローラーでパスを開き、対象自体を選択状態にする
/// (Quick Launch の `Ctrl+E`)。フォルダなら中身を、ファイルなら
/// 親フォルダを開いて選択する — `explorer.exe /select,` の標準動作。
pub fn reveal_in_explorer(path: &str) -> std::io::Result<()> {
    if !Path::new(path).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("path not found: {path}"),
        ));
    }
    // 引数はカンマの後ろにパスをそのまま続ける独自構文で、通常の
    // コマンドライン引数分割 (スペース区切り) には従わない。
    // `ShellExecuteW` の parameters へ 1 本の文字列として渡す
    let args = HSTRING::from(format!("/select,\"{path}\""));
    let result = unsafe {
        ShellExecuteW(
            None,
            None,
            &HSTRING::from("explorer.exe"),
            &args,
            None,
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW failed with code {code}"
        )))
    }
}

/// 管理者として実行する (Quick Launch の `Ctrl+Alt+Enter`、FR-9.8.4)。
/// verb に `runas` を渡すと UAC の同意ダイアログは Windows が出すため、
/// waypoint 側では確認を挟まない。
pub fn run_as_admin(path: &str) -> std::io::Result<()> {
    let target = HSTRING::from(path);
    let result = unsafe { ShellExecuteW(None, w!("runas"), &target, None, None, SW_SHOWNORMAL) };
    let code = result.0 as isize;
    if code > 32 {
        return Ok(());
    }
    // 同意を拒否されただけなら失敗として扱わない。呼び出し側が
    // エラー表示しないための区別 (ERROR_CANCELLED = 1223)
    if code == ERROR_CANCELLED.0 as isize {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "ShellExecuteW runas failed with code {code}"
    )))
}

/// `This PC` など、ファイルシステム上のパスを持たないシェル項目を開く。
pub fn open_shell_item(target: &str) -> std::io::Result<()> {
    let target = HSTRING::from(target);
    let result = unsafe { ShellExecuteW(None, None, &target, None, None, SW_SHOWNORMAL) };
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW failed with code {code}"
        )))
    }
}

/// Windows の既定のフォルダーハンドラーで開く。
fn open_new_window(path: &str) -> std::io::Result<()> {
    open_shell_item(path)
}

/// 既存のエクスプローラーウィンドウのフォルダを変更する。
///
/// `origin` と同じ HWND を持つシェルウィンドウを探し、`Navigate` する。
/// 見つからなければ Err を返して呼び出し側でフォールバックさせる。
fn navigate_existing(origin: HWND, path: &str) -> windows::core::Result<()> {
    unsafe {
        let windows_col: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let count = windows_col.Count()?;

        for i in 0..count {
            let Ok(dispatch) = windows_col.Item(&VARIANT::from(i)) else {
                continue;
            };
            let Ok(browser) = dispatch.cast::<IWebBrowser2>() else {
                continue;
            };
            // エクスプローラーのウィンドウハンドルが一致するものを探す
            let Ok(hwnd) = browser.HWND() else {
                continue;
            };
            if hwnd.0 != origin.0 as isize {
                continue;
            }

            let url = BSTR::from(HSTRING::from(path).to_string());
            browser.Navigate(
                &url,
                Some(&VARIANT::default()),
                Some(&VARIANT::default()),
                Some(&VARIANT::default()),
                Some(&VARIANT::default()),
            )?;
            let _ = SW_SHOWNORMAL; // 表示状態は変更しない
            return Ok(());
        }

        Err(windows::core::Error::empty())
    }
}
