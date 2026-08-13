//! フォルダを開く。新規ウィンドウと、既存ウィンドウのフォルダ変更の 2 通り。

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{BSTR, HSTRING, Interface};

use crate::config::OpenMode;

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

/// 新しいエクスプローラーウィンドウで開く。
fn open_new_window(path: &str) -> std::io::Result<()> {
    // ShellExecuteW でも良いが、explorer.exe に渡すほうが
    // 引数のエスケープを自前で考えずに済む
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
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
