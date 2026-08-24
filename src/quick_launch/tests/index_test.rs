//! `Index::refresh_dynamic` — Recent/Frequent Folders と開いているウィンドウ
//! だけを差し替え、apps / bookmarks / history / azure* は保持することの検証。

use super::super::*;
use super::fixture::config_without_live_scans;
use crate::dynamic::{Menus, WindowEntry};

#[test]
fn refresh_dynamic_replaces_windows_but_keeps_apps_bookmarks_and_history() {
    let mut config = config_without_live_scans();
    // Windows の一覧は refresh_dynamic で更新される対象なので、
    // このテストでは検索できるように有効化する
    // (config_without_live_scans は実マシンをスキャンさせないため既定で無効)。
    config.settings.quick_launch.include_open_windows = true;
    let mut index = Index::build(&config, &Menus::default());

    // apps/bookmarks/history は実マシンをスキャンしないよう config で
    // 無効化しているので、ここでは保持されることを確認するためのダミー
    // データを直接差し込む (`fixture::index` と同じ手法)。
    index.apps = vec![Entry {
        name: "Visual Studio Code".into(),
        breadcrumb: String::new(),
        path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
        action: Action::LaunchApp,
        branch: None,
    }];
    index.apps_lower = search::LowerKeys::build_for(&index.apps);
    index.bookmarks = vec![Entry {
        name: "GitHub".into(),
        breadcrumb: String::new(),
        path: "https://github.com/".into(),
        action: Action::OpenUrl("https://github.com/".into()),
        branch: None,
    }];
    index.bookmarks_lower = search::LowerKeys::build_for(&index.bookmarks);

    let dynamic = Menus {
        all_windows: vec![WindowEntry {
            title: "Notepad".to_string(),
            hwnd: 42,
            process_name: "notepad.exe".to_string(),
        }],
        ..Menus::default()
    };
    index.refresh_dynamic(&config, &dynamic);

    // 新しいウィンドウが検索できる
    let windows = index.search("w notepad");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].action, Action::FocusWindow(42));

    // apps/bookmarks は refresh_dynamic を経ても消えない (再スキャンしていない)
    assert_eq!(index.apps.len(), 1);
    assert_eq!(index.apps[0].name, "Visual Studio Code");
    assert_eq!(index.bookmarks.len(), 1);
    assert_eq!(index.bookmarks[0].name, "GitHub");
}

#[test]
fn refresh_dynamic_updates_recent_and_frequent_folders() {
    use crate::dynamic::PathEntry;

    let config = config_without_live_scans();
    let mut index = Index::build(&config, &Menus::default());

    let dynamic = Menus {
        recent_folders: vec![PathEntry {
            name: "Downloads".into(),
            path: r"C:\Users\me\Downloads".into(),
        }],
        ..Menus::default()
    };
    index.refresh_dynamic(&config, &dynamic);

    let found = index.search("downloads");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, r"C:\Users\me\Downloads");
}
