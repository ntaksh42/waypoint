//! ワークスペース URI を元のエディターへ渡す。

use crate::editor_workspaces::Editor;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

pub fn open_editor_workspace(editor: Editor, uri: &str, is_file: bool) -> std::io::Result<()> {
    let program = editor_executable(editor).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("editor not found: {}", editor.label()),
        )
    })?;
    // cmd ラッパーへ履歴由来の URI を載せず、実体の exe へ直接渡す。
    std::process::Command::new(program)
        .args([
            if is_file {
                "--file-uri"
            } else {
                "--folder-uri"
            },
            uri,
        ])
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map(|_| ())
}

fn editor_executable(editor: Editor) -> Option<PathBuf> {
    if let Some(program) = super::launch::find_executable(editor.command()) {
        if program
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Some(program);
        }
        if let Some(exe) = program
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join(format!("{}.exe", editor.label())))
            .filter(|exe| exe.is_file())
        {
            return Some(exe);
        }
    }
    let install = match editor {
        Editor::Code => "Microsoft VS Code",
        Editor::Cursor => "cursor",
    };
    [
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Programs")),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .map(|root| root.join(install).join(format!("{}.exe", editor.label())))
    .find(|exe| exe.is_file())
}
