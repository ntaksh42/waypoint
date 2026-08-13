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
fn missing_optional_fields_use_defaults() {
    // 最小限の JSON でも既定値で埋まること
    let cfg: Config = serde_json::from_str(r#"{ "items": [] }"#).unwrap();
    assert_eq!(cfg.version, 1);
    assert!(cfg.settings.trigger.middle_click);
    // 既定は Ctrl+Alt+W。Win+W は Windows 11 の Widgets が予約済みで
    // RegisterHotKey が失敗するため使えない
    assert_eq!(cfg.settings.trigger.hotkey, "Ctrl+Alt+W");
    assert!(
        cfg.settings
            .trigger
            .excluded_processes
            .contains(&"chrome.exe".to_string())
    );
    assert_eq!(cfg.settings.menu.icon_size, 16);
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
