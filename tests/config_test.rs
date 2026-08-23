//! 設定の解析・変数展開のテスト。純ロジックのみを対象にする。

use std::collections::BTreeMap;

use waypoint::config::{Config, Item, OpenMode, expand};

fn vars() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("Proj".to_string(), r"D:\work\projects".to_string());
    m.insert("Share".to_string(), r"\\server\team".to_string());
    m
}

#[test]
fn expands_user_variable() {
    let got = expand(r"{Proj}\waypoint", &vars()).unwrap();
    assert_eq!(got, r"D:\work\projects\waypoint");
}

#[test]
fn expands_unc_user_variable() {
    let got = expand(r"{Share}\dev", &vars()).unwrap();
    assert_eq!(got, r"\\server\team\dev");
}

#[test]
fn expands_environment_variable() {
    unsafe { std::env::set_var("WAYPOINT_TEST_DIR", r"C:\tmp") };
    let got = expand(r"%WAYPOINT_TEST_DIR%\sub", &vars()).unwrap();
    assert_eq!(got, r"C:\tmp\sub");
}

#[test]
fn expands_both_kinds() {
    unsafe { std::env::set_var("WAYPOINT_TEST_LEAF", "leaf") };
    let got = expand(r"{Proj}\%WAYPOINT_TEST_LEAF%", &vars()).unwrap();
    assert_eq!(got, r"D:\work\projects\leaf");
}

#[test]
fn unknown_user_variable_is_none() {
    // 解決できない変数はグレー表示にするため None を返す (FR-5.4)
    assert!(expand(r"{NoSuchVar}\x", &vars()).is_none());
}

#[test]
fn unknown_env_variable_is_none() {
    assert!(expand(r"%WAYPOINT_NO_SUCH_VAR_XYZ%\x", &vars()).is_none());
}

#[test]
fn unmatched_brace_is_none() {
    assert!(expand(r"{Proj\x", &vars()).is_none());
}

#[test]
fn path_without_variables_passes_through() {
    let got = expand(r"C:\Windows", &vars()).unwrap();
    assert_eq!(got, r"C:\Windows");
}

#[test]
fn parses_spec_example() {
    // 仕様書 05 章の例がそのまま読めること
    let json = r#"{
      "version": 1,
      "variables": { "Proj": "D:\\work" },
      "settings": {
        "trigger": { "middleClick": true, "hotkey": "Win+W",
                     "excludedProcesses": ["chrome.exe"] },
        "menu": { "iconSize": 16, "numericAccelerators": true },
        "startWithWindows": false
      },
      "items": [
        { "type": "folder", "name": "P", "path": "{Proj}", "open": "reuse" },
        { "type": "separator", "name": "dev" },
        { "type": "submenu", "name": "out", "items": [
          { "type": "folder", "name": "Debug", "path": "{Proj}\\d" }
        ]},
        { "type": "specialFolder", "name": "DL", "knownFolder": "Downloads" }
      ]
    }"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.items.len(), 4);
    assert_eq!(cfg.settings.trigger.hotkey, "Win+W");

    match &cfg.items[0] {
        Item::Folder { name, open, .. } => {
            assert_eq!(name, "P");
            assert_eq!(*open, Some(OpenMode::Reuse));
        }
        other => panic!("expected folder, got {other:?}"),
    }
    match &cfg.items[2] {
        Item::Submenu { items, .. } => assert_eq!(items.len(), 1),
        other => panic!("expected submenu, got {other:?}"),
    }
}

#[test]
fn parses_and_round_trips_shell_item() {
    let json = r#"{ "items": [
        { "type": "shell", "name": "This PC", "target": "shell:MyComputerFolder" }
    ] }"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    match &cfg.items[0] {
        Item::Shell { name, target } => {
            assert_eq!(name, "This PC");
            assert_eq!(target, "shell:MyComputerFolder");
        }
        other => panic!("expected shell, got {other:?}"),
    }

    let text = serde_json::to_string(&cfg).unwrap();
    let again: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(again.items, cfg.items);
}

#[test]
fn missing_optional_fields_use_defaults() {
    // 最小限の JSON でも既定値で埋まること
    let cfg: Config = serde_json::from_str(r#"{ "items": [] }"#).unwrap();
    assert_eq!(cfg.version, 1);
    assert!(cfg.settings.trigger.middle_click);
    // 仕様書 (FR-1.2) 通り既定は Win+W。Windows 11 の Widgets が予約済みの
    // 環境では RegisterHotKey が失敗しうるが、その場合はトレイに警告が出る
    assert_eq!(cfg.settings.trigger.hotkey, "Win+W");
    assert!(
        cfg.settings
            .trigger
            .excluded_processes
            .contains(&"chrome.exe".to_string())
    );
    // QAP の既定と同じ 32px (FR-2.3)
    assert_eq!(cfg.settings.menu.icon_size, 32);
    assert_eq!(cfg.settings.quick_launch.hotkey, "Alt+Space");
    assert!(cfg.settings.quick_launch.include_recent_folders);
    assert!(cfg.settings.quick_launch.include_frequent_folders);
    assert!(cfg.settings.quick_launch.include_browser_history);
    assert!(!cfg.settings.quick_launch.azure_devops.enabled);
    assert!(cfg.settings.quick_launch.azure_devops.projects.is_empty());
    assert!(!cfg.settings.quick_launch.search_paths);
    assert_eq!(cfg.settings.quick_launch.visible_results, 12);
}

#[test]
fn round_trips_through_json() {
    let cfg: Config = serde_json::from_str(
        r#"{ "items": [ { "type": "folder", "name": "A", "path": "C:\\a" } ] }"#,
    )
    .unwrap();
    let text = serde_json::to_string(&cfg).unwrap();
    let again: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(again.items, cfg.items);
}

#[test]
fn separator_without_name_is_valid() {
    let cfg: Config = serde_json::from_str(r#"{ "items": [ { "type": "separator" } ] }"#).unwrap();
    assert_eq!(cfg.items[0].label(), None);
}

/// FR-5.4: 解決できない変数を含む項目を洗い出す。
/// サブメニューの中も見る。
#[test]
fn unresolved_items_are_reported_including_submenus() {
    let cfg: Config = serde_json::from_str(
        r#"{
            "variables": { "Known": "C:\\known" },
            "items": [
                { "type": "folder", "name": "OK",      "path": "{Known}\\a" },
                { "type": "folder", "name": "BadUser", "path": "{Missing}\\b" },
                { "type": "specialFolder", "name": "Special", "knownFolder": "Desktop" },
                { "type": "submenu", "name": "Nested", "items": [
                    { "type": "folder", "name": "BadEnv", "path": "%NO_SUCH_ENV_VAR_XYZ%\\c" }
                ]}
            ]
        }"#,
    )
    .unwrap();

    let found = waypoint::config::unresolved_items(&cfg);
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["BadUser", "BadEnv"], "{found:?}");
    // 展開前のパスを返す (何が解決できなかったか分かるように)
    assert_eq!(found[0].1, "{Missing}\\b");
}

/// 解決できる項目しかなければ空。
#[test]
fn unresolved_items_is_empty_when_all_resolve() {
    let cfg: Config = serde_json::from_str(
        r#"{
            "variables": { "P": "C:\\p" },
            "items": [ { "type": "folder", "name": "OK", "path": "{P}\\x" } ]
        }"#,
    )
    .unwrap();
    assert!(waypoint::config::unresolved_items(&cfg).is_empty());
}

/// Quick Launch からの「お気に入り登録」はルートメニュー末尾へ追加する。
#[test]
fn add_item_if_new_appends_to_the_root_menu() {
    let mut cfg = Config::default();
    let added = cfg.add_item_if_new(Item::Folder {
        name: "Docs".to_string(),
        path: r"E:\waypoint\docs".to_string(),
        open: None,
        icon: None,
        show_branch: false,
    });
    assert!(added);
    assert_eq!(cfg.items.len(), 1);
}

/// 同じパスを持つ項目が既にあれば (大文字小文字を区別せず) 追加しない。
/// Quick Launch から重複登録しても config が膨らまないようにするため。
#[test]
fn add_item_if_new_skips_a_path_that_already_exists() {
    let mut cfg = Config {
        items: vec![Item::Folder {
            name: "Docs".to_string(),
            path: r"E:\waypoint\DOCS".to_string(),
            open: None,
            icon: None,
            show_branch: false,
        }],
        ..Config::default()
    };
    let added = cfg.add_item_if_new(Item::Folder {
        name: "Docs (again)".to_string(),
        path: r"e:\waypoint\docs".to_string(),
        open: None,
        icon: None,
        show_branch: false,
    });
    assert!(!added);
    assert_eq!(cfg.items.len(), 1);
}

/// 既存パスの重複チェックはサブメニューの中も見る。
#[test]
fn add_item_if_new_checks_paths_inside_submenus_too() {
    let mut cfg = Config {
        items: vec![Item::Submenu {
            name: "Projects".to_string(),
            items: vec![Item::File {
                name: "Notes".to_string(),
                path: r"E:\notes.txt".to_string(),
                icon: None,
            }],
            show_branch: false,
        }],
        ..Config::default()
    };
    let added = cfg.add_item_if_new(Item::File {
        name: "Notes (again)".to_string(),
        path: r"E:\notes.txt".to_string(),
        icon: None,
    });
    assert!(!added);
}

/// Quick Launch の索引は展開済みパスを持つため、設定内の変数付きパスとも
/// 同じ対象として重複判定する。
#[test]
fn add_item_if_new_compares_expanded_paths() {
    let mut cfg = Config {
        variables: BTreeMap::from([("Work".to_string(), r"E:\work".to_string())]),
        items: vec![Item::Folder {
            name: "Project".to_string(),
            path: r"{Work}\project".to_string(),
            open: None,
            icon: None,
            show_branch: false,
        }],
        ..Config::default()
    };

    let added = cfg.add_item_if_new(Item::Folder {
        name: "Project (again)".to_string(),
        path: r"E:\work\project".to_string(),
        open: None,
        icon: None,
        show_branch: false,
    });

    assert!(!added);
    assert_eq!(cfg.items.len(), 1);
}
