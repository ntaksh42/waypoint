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

/// `Index::refresh_azure` は Azure DevOps の候補だけを組み直し、
/// apps / bookmarks / history / Recent Folders は保持する。
///
/// バックグラウンド同期の完了通知はフル `Index::build` を呼ばない
/// (呼ぶとスタートメニューの再スキャンが同期のたびに UI スレッドで走る)。
/// その前提が崩れていないことを、再スキャンすると消えるダミーデータが
/// 残っているかで確かめる。
#[test]
fn refresh_azure_keeps_apps_bookmarks_history_and_folders() {
    use crate::dynamic::PathEntry;

    let mut config = config_without_live_scans();
    // Azure は無効のまま (実際の API / SQLite を触らせない)。
    // refresh_azure が azure* を空へ組み直すこと自体は問題なく、
    // ここで見たいのは「それ以外が巻き添えで消えないこと」。
    config.settings.quick_launch.azure_devops.enabled = false;

    let dynamic = Menus {
        recent_folders: vec![PathEntry {
            name: "Downloads".into(),
            path: r"C:\Users\me\Downloads".into(),
        }],
        ..Menus::default()
    };
    let mut index = Index::build(&config, &dynamic);

    // 実マシンをスキャンしない設定なので、保持を確かめる対象を直接差し込む
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
    index.history = vec![Entry {
        name: "Rust docs".into(),
        breadcrumb: "Chrome History".into(),
        path: "https://doc.rust-lang.org/".into(),
        action: Action::OpenUrl("https://doc.rust-lang.org/".into()),
        branch: None,
    }];
    index.history_lower = search::LowerKeys::build_for(&index.history);

    index.refresh_azure(&config);

    assert_eq!(index.apps.len(), 1, "apps が再スキャンされて消えている");
    assert_eq!(index.apps[0].name, "Visual Studio Code");
    assert_eq!(index.bookmarks.len(), 1, "bookmarks が消えている");
    assert_eq!(index.bookmarks[0].name, "GitHub");
    assert_eq!(index.history.len(), 1, "history が消えている");
    assert_eq!(index.history[0].name, "Rust docs");

    // Recent Folders (entries 側) も保持される
    let found = index.search("downloads");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, r"C:\Users\me\Downloads");

    // 検索経路からも引けること (lower キーが対で保たれている)
    assert_eq!(index.search("a visual").len(), 1);
    assert_eq!(index.search("b github").len(), 1);
    assert_eq!(index.search("h rust").len(), 1);
}

/// `Index::refresh_config_items` は config 由来の候補を組み直しつつ、
/// apps / bookmarks / history は保持する。
///
/// お気に入り登録 (`Ctrl+Shift+Enter`) の経路。新しく足した項目が
/// すぐ検索できること (これが目的) と、スタートメニューの再スキャンが
/// 起きていないこと (これが最適化) の両方を見る。
#[test]
fn refresh_config_items_adds_new_item_and_keeps_apps() {
    use crate::config::Item;

    let mut config = config_without_live_scans();
    let mut index = Index::build(&config, &Menus::default());

    // 実マシンをスキャンしない設定なので、保持を確かめる対象を直接差し込む
    index.apps = vec![Entry {
        name: "Visual Studio Code".into(),
        breadcrumb: String::new(),
        path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
        action: Action::LaunchApp,
        branch: None,
    }];
    index.apps_lower = search::LowerKeys::build_for(&index.apps);

    // お気に入り登録に相当する config の変更
    config.items.push(Item::Folder {
        name: "Waypoint source".into(),
        path: r"E:\waypoint\src".into(),
        open: None,
        icon: None,
        show_branch: false,
    });
    index.refresh_config_items(&config, &Menus::default());

    // 足した項目がすぐ検索できる
    let found = index.search("waypoint source");
    assert_eq!(found.len(), 1, "追加した項目が検索できない");
    assert_eq!(found[0].path, r"E:\waypoint\src");

    // apps は再スキャンされていない
    assert_eq!(index.apps.len(), 1, "apps が再スキャンされて消えている");
    assert_eq!(index.search("a visual").len(), 1);
}
