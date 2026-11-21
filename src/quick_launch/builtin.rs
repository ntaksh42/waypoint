//! 通常検索に載せる waypoint 自身の固定候補 (FR-9.20 / 9.22 / 9.28)。

use super::search::LowerKeys;
use super::{Action, Entry};

/// waypoint 自身の操作を通常検索へ載せる固定候補 (FR-9.20)。
///
/// config や外部データに依存しない静的な一覧なので、`Index` の
/// 構築・更新経路には乗せず `azure_command_entries` と同じ形で
/// 静的に持つ。`LowerKeys` も一度だけ作って使い回す。
pub(crate) fn builtin_command_entries() -> (&'static [Entry], &'static [LowerKeys]) {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        vec![
            Entry {
                azure: None,
                name: "Settings".to_string(),
                breadcrumb: "Open waypoint settings".to_string(),
                path: String::new(),
                action: Action::OpenSettings,
                branch: None,
            },
            Entry {
                azure: None,
                name: "Help".to_string(),
                breadcrumb: "Open waypoint help".to_string(),
                path: String::new(),
                action: Action::OpenHelp,
                branch: None,
            },
            // FR-9.28: Edge のお気に入り編集画面 (設定画面の別モード)
            Entry {
                azure: None,
                name: "Edit Bookmarks".to_string(),
                breadcrumb: "Edit Edge bookmarks".to_string(),
                path: String::new(),
                action: Action::OpenBookmarkEditor,
                branch: None,
            },
        ]
    });
    static LOWER: std::sync::LazyLock<Vec<LowerKeys>> =
        std::sync::LazyLock::new(|| LowerKeys::build_for_names(&ENTRIES));
    (&ENTRIES, &LOWER)
}
