//! 機能一覧の Help ページを開く。トレイメニューと Quick Launch の
//! 内部コマンド (FR-9.22) の両方から呼ばれる。

use std::path::{Path, PathBuf};

/// exe と同じディレクトリの `help\index.html` を既定ブラウザで開く。
pub fn open() -> std::io::Result<()> {
    let help = help_page_path(&std::env::current_exe()?);
    crate::shell::open_shell_item(&help.to_string_lossy())
}

fn help_page_path(waypoint_exe: &Path) -> PathBuf {
    waypoint_exe
        .parent()
        .expect("waypoint executable always has a parent")
        .join("help")
        .join("index.html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_page_is_bundled_next_to_the_executable() {
        assert_eq!(
            help_page_path(Path::new(r"C:\Program Files\waypoint\waypoint.exe")),
            PathBuf::from(r"C:\Program Files\waypoint\help\index.html")
        );
    }
}
