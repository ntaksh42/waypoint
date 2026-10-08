//! 外部 CLI / エディタでフォルダを開く (`ps ` / `ed ` / `cc ` / `codex `)。

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use windows::Win32::System::Threading::CREATE_NO_WINDOW;

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

    powershell_fallback(path, None).spawn().map(|_| ())
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
    // CREATE_NO_WINDOW を付けないとコンソールが一瞬開いて閉じる。
    // フォルダ名の `&` や `%` を cmd.exe に解釈させないよう、パスはコマンド
    // ラインへ載せず作業ディレクトリで渡して引数は `.` にする
    if is_batch_script(&program) {
        return std::process::Command::new("cmd.exe")
            .arg("/c")
            .arg(&program)
            .arg(".")
            .current_dir(path)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .map(|_| ());
    }
    std::process::Command::new(&program)
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// Claude Code を Windows Terminal + PowerShell 7 で指定フォルダ・表示名付きで起動する。
/// `session_name` は省略でき、その場合は表示名を付けずに起動する。PowerShell 7 が
/// 見つからない場合は Windows 標準の `powershell.exe` (5.1) にフォールバックする。
pub fn open_claude_code(path: &str, session_name: Option<&str>) -> std::io::Result<()> {
    if !Path::new(path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {path}"),
        ));
    }
    let command = match session_name {
        Some(session_name) => format!("& claude --name '{}'", escape_single_quoted(session_name)),
        None => "& claude".to_string(),
    };
    run_in_terminal(path, &command)
}

/// Claude Code / Codex の過去セッションを作業フォルダで再開する (`cs `、FR-9.15.6)。
/// Claude Code はセッションを作業フォルダ単位で保存しているため、
/// 記録に残っている作業フォルダで起動しないと id を見つけられない。
pub fn resume_agent_session(
    path: &str,
    agent: crate::agent_sessions::Agent,
    id: &str,
) -> std::io::Result<()> {
    if !Path::new(path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {path}"),
        ));
    }
    let id = escape_single_quoted(id);
    let command = match agent {
        crate::agent_sessions::Agent::ClaudeCode => format!("& claude --resume '{id}'"),
        crate::agent_sessions::Agent::Codex => format!("& codex resume '{id}'"),
    };
    run_in_terminal(path, &command)
}

/// Windows Terminal + PowerShell 7 で `path` を作業ディレクトリにして `command` を
/// 実行する。見つからなければ Windows 標準の `powershell.exe` (5.1) で開く。
fn run_in_terminal(path: &str, command: &str) -> std::io::Result<()> {
    if let Some(pwsh) = find_pwsh()
        && std::process::Command::new("wt.exe")
            .args(["-d", path])
            .arg(&pwsh)
            .args(["-NoExit", "-Command", command])
            .spawn()
            .is_ok()
    {
        return Ok(());
    }

    powershell_fallback(path, Some(command)).spawn().map(|_| ())
}

/// Codex CLI を Windows Terminal + PowerShell 7 で指定フォルダを作業ディレクトリ
/// にして起動する (`cx ` プレフィックス、FR-9.15.5)。`open_terminal` と同じ理由
/// (`wt.exe` の PATH 解決文脈の違い) で `pwsh.exe` のフルパスを自前で解決し、
/// PowerShell 7 が見つからなければ Windows 標準の `powershell.exe` (5.1) に
/// フォールバックする。
///
/// Codex statusline が配置済みなら、そのラッパーが Windows Terminal と
/// ステータス表示ペインをまとめて開く。未配置時は従来の Codex CLI を起動する。
pub fn open_codex(path: &str) -> std::io::Result<()> {
    if !Path::new(path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("folder not found: {path}"),
        ));
    }

    if let Some(wrapper) = std::env::var_os("LOCALAPPDATA")
        .map(|dir| {
            PathBuf::from(dir)
                .join("CodexStatusline")
                .join("codex-wt.ps1")
        })
        .filter(|wrapper| wrapper.is_file())
        && let Some(pwsh) = find_pwsh()
        && std::process::Command::new(pwsh)
            .args(["-NoProfile", "-File"])
            .arg(wrapper)
            .current_dir(path)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .is_ok()
    {
        return Ok(());
    }

    let codex = find_executable("codex")
        .map(|program| program.display().to_string())
        .unwrap_or_else(|| "codex".to_string());
    let command = format!("& '{}'", escape_single_quoted(&codex));

    if let Some(pwsh) = find_pwsh()
        && std::process::Command::new("wt.exe")
            .args(["-d", path])
            .arg(&pwsh)
            .args(["-NoExit", "-Command", &command])
            .spawn()
            .is_ok()
    {
        return Ok(());
    }

    powershell_fallback(path, Some(&command))
        .spawn()
        .map(|_| ())
}

/// PowerShell の単一引用符文字列へ埋め込む値をエスケープする。
/// PowerShell は ASCII の `'` のほかに `‘ ’ ‚ ‛` も単一引用符として扱うため、
/// 4 種類すべてを二重化しないと文字列から抜けられてしまう。
fn escape_single_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        escaped.push(ch);
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            escaped.push(ch);
        }
    }
    escaped
}

/// PowerShell 7 が無い環境で使う Windows PowerShell の起動コマンド。
fn powershell_fallback(path: &str, command: Option<&str>) -> std::process::Command {
    let mut process = std::process::Command::new("powershell.exe");
    // Windows PowerShell 5.1 は -WorkingDirectory を反映しないため、
    // プロセスの作業ディレクトリとして渡す。
    process.arg("-NoExit").current_dir(path);
    if let Some(command) = command {
        process.args(["-Command", command]);
    }
    process
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_fallback_runs_in_requested_directory() {
        let path = format!("{}\\src", env!("CARGO_MANIFEST_DIR"));
        let output = powershell_fallback(&path, Some("(Get-Location).Path; exit 0"))
            .creation_flags(CREATE_NO_WINDOW.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), path);
    }

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
    fn single_quote_variants_are_doubled() {
        assert_eq!(escape_single_quoted("it's"), "it''s");
        assert_eq!(
            escape_single_quoted("x\u{2019}; calc; \u{2018}"),
            "x\u{2019}\u{2019}; calc; \u{2018}\u{2018}"
        );
        assert_eq!(escape_single_quoted("plain"), "plain");
    }

    /// フォルダ名の `&` が cmd.exe にコマンド区切りとして解釈されないこと。
    /// パスは引数でなく作業ディレクトリで渡す (エディターには `.` が届く)
    #[test]
    fn batch_editor_does_not_put_the_path_on_the_command_line() {
        let base = std::env::temp_dir().join("waypoint_editor_inject");
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("a&hostname&b");
        std::fs::create_dir_all(&folder).unwrap();
        let report = base.join("report.txt");
        std::fs::write(
            base.join("fakeeditor.cmd"),
            "@echo off\r\necho %~1> \"%~dp0report.txt\"\r\ncd >> \"%~dp0report.txt\"\r\n",
        )
        .unwrap();

        open_editor(
            base.join("fakeeditor.cmd").to_str().unwrap(),
            folder.to_str().unwrap(),
        )
        .unwrap();
        for _ in 0..30 {
            if std::fs::read_to_string(&report).is_ok_and(|text| text.lines().count() >= 2) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let text = std::fs::read_to_string(&report).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("."));
        assert_eq!(lines.next(), Some(folder.to_str().unwrap()));
        assert_eq!(lines.next(), None, "余計なコマンドが実行された: {text}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn batch_scripts_are_detected_case_insensitively() {
        assert!(is_batch_script(Path::new(r"C:\bin\code.CMD")));
        assert!(is_batch_script(Path::new(r"C:\bin\run.bat")));
        assert!(!is_batch_script(Path::new(r"C:\bin\idea64.exe")));
    }
}
