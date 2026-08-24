use super::super::azure::AzureIndexed;
use super::super::*;
use crate::config::Config;

/// 実マシンの Open Windows / ブックマーク / 履歴 / インストール済みアプリを
/// スキャンしない `Config`。`Config::default()` は Quick Launch の該当設定が
/// 既定でオン (FR-9.2) のため、`Index::build` に渡すとテスト実行環境依存の
/// 結果が紛れ込む。これらの経路を検証しないテストはこちらを使う。
pub(super) fn config_without_live_scans() -> Config {
    let mut config = Config::default();
    let quick_launch = &mut config.settings.quick_launch;
    quick_launch.include_open_windows = false;
    quick_launch.include_bookmarks = false;
    quick_launch.include_browser_history = false;
    quick_launch.include_apps = false;
    config
}

pub(super) fn index() -> Index {
    let mut index = Index {
        entries: vec![
            Entry {
                name: "Release".into(),
                breadcrumb: "Projects > waypoint".into(),
                path: r"E:\waypoint\target\release".into(),
                action: Action::OpenFolder(OpenMode::Reuse),
                branch: None,
            },
            Entry {
                name: "Waypoint docs".into(),
                breadcrumb: "Projects".into(),
                path: r"E:\waypoint\docs".into(),
                action: Action::OpenFolder(OpenMode::NewWindow),
                branch: None,
            },
            Entry {
                name: "Old waypoint".into(),
                breadcrumb: "Archive".into(),
                path: r"E:\archive\waypoint".into(),
                action: Action::OpenFolder(OpenMode::NewWindow),
                branch: None,
            },
        ],
        bookmarks: vec![
            Entry {
                name: "GitHub".into(),
                breadcrumb: "Work".into(),
                path: "https://github.com/".into(),
                action: Action::OpenUrl("https://github.com/".into()),
                branch: None,
            },
            Entry {
                name: "Example".into(),
                breadcrumb: String::new(),
                path: "https://example.com/".into(),
                action: Action::OpenUrl("https://example.com/".into()),
                branch: None,
            },
        ],
        history: vec![Entry {
            name: "WayPoint pull request".into(),
            breadcrumb: "Chrome History".into(),
            path: "https://github.com/example/waypoint/pull/1".into(),
            action: Action::OpenUrl("https://github.com/example/waypoint/pull/1".into()),
            branch: None,
        }],
        azure: {
            let entry = Entry {
                name: "PR 42: Add Azure search".into(),
                breadcrumb: "Azure DevOps — org/Waypoint — active — wp".into(),
                path: "https://dev.azure.com/org/Waypoint/_git/app/pullrequest/42".into(),
                action: Action::OpenUrl(
                    "https://dev.azure.com/org/Waypoint/_git/app/pullrequest/42".into(),
                ),
                branch: None,
            };
            vec![AzureIndexed {
                lower: super::super::search::LowerKeys::new(&entry),
                entry,
                kind: crate::azure_devops::Kind::PullRequest,
                status: "active".into(),
                is_mine: true,
            }]
        },
        azure_work_items: Vec::new(),
        azure_work_items_lower: Vec::new(),
        windows: vec![Entry {
            name: "waypoint - Notepad".into(),
            breadcrumb: "Open Windows".into(),
            path: String::new(),
            action: Action::FocusWindow(12345),
            branch: None,
        }],
        apps: vec![Entry {
            name: "Visual Studio Code".into(),
            breadcrumb: String::new(),
            path: r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk"
                .into(),
            action: Action::LaunchApp,
            branch: None,
        }],
        search_paths: false,
        ranking: Ranking::default(),
        ..Default::default()
    };
    index.entries_lower = super::super::search::LowerKeys::build_for(&index.entries);
    index.bookmarks_lower = super::super::search::LowerKeys::build_for(&index.bookmarks);
    index.history_lower = super::super::search::LowerKeys::build_for(&index.history);
    index.windows_lower = super::super::search::LowerKeys::build_for(&index.windows);
    index.apps_lower = super::super::search::LowerKeys::build_for(&index.apps);
    index
}
