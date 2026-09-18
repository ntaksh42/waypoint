//! フォルダを開く。新規ウィンドウと、既存ウィンドウのフォルダ変更の 2 通り。

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{AttachThreadInput, CREATE_NO_WINDOW, GetCurrentThreadId};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellExecuteW, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{BSTR, HSTRING, Interface, w};

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

/// フォルダを指定されたエディターで開く (`ed ` プレフィックス)。
pub fn open_editor(command: &str, path: &str) -> std::io::Result<()> {
    if !Path::new(path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {path}"),
        ));
    }
    let program = find_executable(command).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("editor not found: {command}"),
        )
    })?;

    // `.cmd` / `.bat` は CreateProcessW が直接起動できないので cmd.exe を挟む。
    // VS Code が PATH へ置くのは `code.cmd` なので、既定値がこちらに来る。
    // CREATE_NO_WINDOW を付けないとコンソールが一瞬開いて閉じる
    if is_batch_script(&program) {
        return std::process::Command::new("cmd.exe")
            .arg("/c")
            .arg(&program)
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .map(|_| ());
    }
    std::process::Command::new(&program)
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// Claude Code を Windows Terminal で指定フォルダ・表示名付きで起動する。
/// `session_name` は省略でき、その場合は表示名を付けずに起動する。
pub fn open_claude_code(path: &str, session_name: Option<&str>) -> std::io::Result<()> {
    if !Path::new(path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {path}"),
        ));
    }
    let mut wt_args = vec!["-d", path, "claude"];
    if let Some(session_name) = session_name {
        wt_args.extend(["--name", session_name]);
    }
    if std::process::Command::new("wt.exe")
        .args(&wt_args)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    let command = match session_name {
        Some(session_name) => format!("& claude --name '{}'", session_name.replace('\'', "''")),
        None => "& claude".to_string(),
    };
    std::process::Command::new("powershell.exe")
        .args(["-NoExit", "-WorkingDirectory", path, "-Command", &command])
        .spawn()
        .map(|_| ())
}

/// 実行ファイル名から実体のフルパスを解決する。パス区切りを含む指定は
/// そのまま、名前だけの指定は `PATH` × `PATHEXT` の総当たりで探す。
///
/// `std::process::Command` の実行ファイル探索は `CreateProcessW` 任せで、
/// `PATHEXT` を見ない。VS Code が PATH へ置くのは `code.cmd` だけなので、
/// 既定の `code` が「program not found」で黙って落ちていた。
fn find_executable(command: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    find_executable_in(command, std::env::split_paths(&paths))
}

/// `find_executable` の探索本体。テストから `PATH` を差し替えずに叩けるよう、
/// 探索対象のディレクトリを引数で受ける。
fn find_executable_in(
    command: &str,
    search_dirs: impl Iterator<Item = PathBuf>,
) -> Option<PathBuf> {
    let as_path = Path::new(command);
    if as_path.components().count() > 1 {
        return with_extensions(as_path).find(|candidate| candidate.is_file());
    }

    search_dirs
        .flat_map(|dir| with_extensions(&dir.join(command)).collect::<Vec<_>>())
        .find(|candidate| candidate.is_file())
}

/// `PATHEXT` の各拡張子を付けた候補を返す。`base` が既に拡張子を持つ場合は
/// それ自体も先頭の候補にする。
///
/// 拡張子の無い `base` 自体は候補にしない。VS Code の bin には Windows では
/// 起動できない拡張子なしの `code` (sh スクリプト) が `code.cmd` と並んで
/// 置かれており、先に拾うと有効な Win32 アプリケーションでないと言われる。
fn with_extensions(base: &Path) -> impl Iterator<Item = PathBuf> + use<> {
    let explicit = base
        .extension()
        .is_some()
        .then(|| base.to_path_buf())
        .into_iter();
    let pathext = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| {
            let mut with_ext = base.as_os_str().to_os_string();
            with_ext.push(ext);
            PathBuf::from(with_ext)
        })
        .collect::<Vec<_>>();
    explicit.chain(pathext)
}

/// 拡張子が `.cmd` / `.bat` か。`CreateProcessW` が直接起動できない形式。
fn is_batch_script(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// VS Code が PATH へ置くのは `code.cmd` だけ (拡張子なしの `code` は
    /// sh スクリプト)。`Command::new("code")` は CreateProcessW が PATHEXT を
    /// 見ないため「program not found」で黙って落ちていた
    #[test]
    fn find_executable_resolves_cmd_from_path() {
        let dir = std::env::temp_dir().join("waypoint_find_executable_cmd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 拡張子なしのスクリプトも一緒に置き、`.cmd` が選ばれることを見る
        std::fs::write(dir.join("dummyeditor"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("dummyeditor.cmd"), "@echo off\n").unwrap();

        let found = find_executable_in("dummyeditor", std::iter::once(dir.clone()));

        // 付ける拡張子は PATHEXT の綴り (既定は大文字) をそのまま使う
        assert!(
            found.as_ref().is_some_and(|found| found
                .as_os_str()
                .eq_ignore_ascii_case(dir.join("dummyeditor.cmd").as_os_str())),
            "unexpected: {found:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_executable_accepts_absolute_path() {
        let dir = std::env::temp_dir().join("waypoint_find_executable_abs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("someeditor.exe");
        std::fs::write(&exe, "").unwrap();

        assert_eq!(find_executable(exe.to_str().unwrap()), Some(exe.clone()));
        // 拡張子を省いた絶対パスも PATHEXT で補える (綴りは PATHEXT のまま)
        let completed = find_executable(dir.join("someeditor").to_str().unwrap());
        assert!(
            completed
                .as_ref()
                .is_some_and(|found| found.as_os_str().eq_ignore_ascii_case(exe.as_os_str())),
            "unexpected: {completed:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_executable_returns_none_when_missing() {
        assert!(
            find_executable_in(
                "waypoint_no_such_editor_xyz",
                std::iter::once(std::env::temp_dir())
            )
            .is_none()
        );
    }

    #[test]
    fn batch_scripts_are_detected_case_insensitively() {
        assert!(is_batch_script(Path::new(r"C:\bin\code.CMD")));
        assert!(is_batch_script(Path::new(r"C:\bin\run.bat")));
        assert!(!is_batch_script(Path::new(r"C:\bin\idea64.exe")));
    }
}
