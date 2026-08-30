//! フォルダを開く。新規ウィンドウと、既存ウィンドウのフォルダ変更の 2 通り。

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellExecuteW, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};
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

/// Current Windows で選んだウィンドウを復元して前面へ移す。
///
/// `SetForegroundWindow` は、呼び出し元スレッドが対象ウィンドウと異なる
/// フォアグラウンド系列に属する場合、Windows のフォーカス窃取防止規則に
/// より無視されタスクバーが点滅するだけになる (Quick Launch / トレイ
/// メニューいずれも waypoint 自身のスレッドから呼ぶため、対象が別スレッド
/// なら毎回この状況になる)。現在のフォアグラウンドスレッドへ
/// `AttachThreadInput` で一時的に入力を結合すると回避できる。
pub fn activate_window(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        let foreground = GetForegroundWindow();
        if !IsWindow(Some(foreground)).as_bool() {
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            return;
        }

        let mut fg_pid = 0u32;
        let foreground_thread = GetWindowThreadProcessId(foreground, Some(&mut fg_pid));
        let mut target_pid = 0u32;
        let target_thread = GetWindowThreadProcessId(hwnd, Some(&mut target_pid));
        let current_thread = GetCurrentThreadId();

        if foreground_thread == 0 || target_thread == 0 || foreground_thread == target_thread {
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            return;
        }

        let mut attached = Vec::new();
        for thread in [foreground_thread, target_thread] {
            if thread != current_thread && AttachThreadInput(current_thread, thread, true).as_bool()
            {
                attached.push(thread);
            }
        }

        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);

        for thread in attached.into_iter().rev() {
            let _ = AttachThreadInput(current_thread, thread, false);
        }
    }
}

/// フォルダを Windows Terminal + PowerShell 7 でカレントディレクトリとして開く
/// (`ps ` プレフィックス、FR-9.15.1)。
///
/// `wt.exe` (パッケージ化アプリの App Execution Alias) に渡すコマンドラインは
/// 通常のプロセスと PATH 解決の文脈が異なり、裸の `pwsh` では
/// `ERROR_FILE_NOT_FOUND` になることを実機で確認済み。`pwsh.exe` のフルパスを
/// 自前で解決してから渡す。`wt.exe` または `pwsh.exe` が見つからない場合は
/// Windows 標準の `powershell.exe` (5.1) にフォールバックする。
pub fn open_terminal(path: &str) -> std::io::Result<()> {
    if !Path::new(path).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("path not found: {path}"),
        ));
    }

    if let Some(pwsh) = find_pwsh()
        && std::process::Command::new("wt.exe")
            .args(["-d", path])
            .arg(&pwsh)
            .spawn()
            .is_ok()
    {
        return Ok(());
    }

    std::process::Command::new("powershell.exe")
        .args(["-NoExit", "-WorkingDirectory", path])
        .spawn()
        .map(|_| ())
}

/// PowerShell 7 (`pwsh.exe`) のフルパスを探す。既定のインストール先を先に見て、
/// 無ければ `PATH` から探す (winget/MSI どちらでインストールしても既定は前者)。
fn find_pwsh() -> Option<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles")?;
    let default_path = Path::new(&program_files).join(r"PowerShell\7\pwsh.exe");
    if default_path.is_file() {
        return Some(default_path);
    }

    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("pwsh.exe"))
            .find(|candidate| candidate.is_file())
    })
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
