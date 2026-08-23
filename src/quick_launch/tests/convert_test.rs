use super::super::*;
use crate::config::Config;
use crate::dynamic::Menus;

/// showBranch が真の Folder は、このリポジトリ自身を指せば
/// Entry::branch にブランチ名が入る (FR-2.14 相当) 。
#[test]
fn folder_with_show_branch_carries_branch_name() {
    let config = Config {
        items: vec![Item::Folder {
            name: "waypoint".to_string(),
            path: env!("CARGO_MANIFEST_DIR").to_string(),
            open: None,
            icon: None,
            show_branch: true,
        }],
        ..Config::default()
    };
    let index = Index::build(&config, &Menus::default());
    let found = index.search("waypoint");
    assert_eq!(found.len(), 1);
    assert!(found[0].branch.is_some());
}

/// showBranch が偽なら、同じパスでもブランチ名を読まない (オプトイン) 。
#[test]
fn folder_without_show_branch_has_no_branch() {
    let config = Config {
        items: vec![Item::Folder {
            name: "waypoint".to_string(),
            path: env!("CARGO_MANIFEST_DIR").to_string(),
            open: None,
            icon: None,
            show_branch: false,
        }],
        ..Config::default()
    };
    let index = Index::build(&config, &Menus::default());
    let found = index.search("waypoint");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].branch, None);
}

/// Submenu の showBranch は配下の Folder (自身は showBranch=false) へ
/// 継承される。
#[test]
fn submenu_show_branch_is_inherited_by_child_folders() {
    let config = Config {
        items: vec![Item::Submenu {
            name: "Repos".to_string(),
            items: vec![Item::Folder {
                name: "waypoint".to_string(),
                path: env!("CARGO_MANIFEST_DIR").to_string(),
                open: None,
                icon: None,
                show_branch: false,
            }],
            show_branch: true,
        }],
        ..Config::default()
    };
    let index = Index::build(&config, &Menus::default());
    let found = index.search("waypoint");
    assert_eq!(found.len(), 1);
    assert!(found[0].branch.is_some());
}

#[test]
fn open_folder_entry_converts_to_folder_item_with_same_open_mode() {
    let entry = Entry {
        name: "Docs".into(),
        breadcrumb: String::new(),
        path: r"E:\waypoint\docs".into(),
        action: Action::OpenFolder(OpenMode::Reuse),
        branch: None,
    };
    let item = entry.to_item().expect("folder entries are addable");
    assert_eq!(
        item,
        Item::Folder {
            name: "Docs".into(),
            path: r"E:\waypoint\docs".into(),
            open: Some(OpenMode::Reuse),
            icon: None,
            show_branch: false,
        }
    );
}

/// FocusWindow / OpenUrl は config へ永続化できないので、
/// お気に入り登録の対象外として None を返す。
#[test]
fn window_and_url_entries_are_not_addable() {
    let window = Entry {
        name: "Notepad".into(),
        breadcrumb: String::new(),
        path: String::new(),
        action: Action::FocusWindow(1),
        branch: None,
    };
    let url = Entry {
        name: "Example".into(),
        breadcrumb: String::new(),
        path: "https://example.com/".into(),
        action: Action::OpenUrl("https://example.com/".into()),
        branch: None,
    };
    assert!(window.to_item().is_none());
    assert!(url.to_item().is_none());
}

/// Everything の結果 (`OpenWithDefaultHandler`) は実際のパスを見て
/// フォルダかファイルかを判定する。このリポジトリ自身を使って確認する。
#[test]
fn everything_result_becomes_folder_item_when_path_is_a_directory() {
    let entry = Entry {
        name: "src".into(),
        breadcrumb: String::new(),
        path: format!("{}\\src", env!("CARGO_MANIFEST_DIR")),
        action: Action::OpenWithDefaultHandler,
        branch: None,
    };
    let item = entry.to_item().expect("existing directories are addable");
    assert!(matches!(item, Item::Folder { .. }));
}

#[test]
fn everything_result_becomes_file_item_when_path_is_not_a_directory() {
    let entry = Entry {
        name: "Cargo.toml".into(),
        breadcrumb: String::new(),
        path: format!("{}\\Cargo.toml", env!("CARGO_MANIFEST_DIR")),
        action: Action::OpenWithDefaultHandler,
        branch: None,
    };
    let item = entry.to_item().expect("existing files are addable");
    assert!(matches!(item, Item::File { .. }));
}

#[test]
fn launch_app_entry_converts_to_file_item() {
    let entry = Entry {
        name: "Visual Studio Code".into(),
        breadcrumb: String::new(),
        path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
        action: Action::LaunchApp,
        branch: None,
    };
    let item = entry.to_item().expect("apps are addable");
    assert_eq!(
        item,
        Item::File {
            name: "Visual Studio Code".into(),
            path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
            icon: None,
        }
    );
}
