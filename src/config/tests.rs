//! `config` のテスト。本体が 500 行規約を超えたため分離した。

use crate::config::{Config, MonitorChoice, merge_shared_settings_text, write_atomic};

#[test]
fn atomic_write_replaces_an_existing_file() {
    let root = std::env::temp_dir().join(format!(
        "waypoint-atomic-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("data.json");

    write_atomic(&path, "first").unwrap();
    write_atomic(&path, "second").unwrap();
    write_atomic(&path, "third").unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "third");
    assert_eq!(
        std::fs::read_to_string(root.join("data.bak.json")).unwrap(),
        "second"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_settings_override_quick_launch_hotkey_from_config() {
    let mut cfg = Config::default();
    assert_eq!(cfg.settings.quick_launch.hotkey, "Alt+Space");

    let shared_text = serde_json::to_string(&{
        let mut settings = cfg.settings.clone();
        settings.quick_launch.hotkey = "Ctrl+Alt+Space".to_string();
        settings
    })
    .unwrap();

    merge_shared_settings_text(&mut cfg, &shared_text);

    assert_eq!(cfg.settings.quick_launch.hotkey, "Ctrl+Alt+Space");
}

#[test]
fn invalid_shared_settings_text_leaves_config_untouched() {
    let mut cfg = Config::default();
    let original_hotkey = cfg.settings.quick_launch.hotkey.clone();

    merge_shared_settings_text(&mut cfg, "not valid json");

    assert_eq!(cfg.settings.quick_launch.hotkey, original_hotkey);
}

/// `monitor` は後から足した設定なので、既存の config.json (キーが無い) を
/// 読んでも既定の `primary` で通ること。ここが壊れるとパース失敗で
/// 設定全体が既定へ戻る。
#[test]
fn monitor_defaults_to_primary_when_absent() {
    let cfg: Config = serde_json::from_str(r#"{"settings":{"quickLaunch":{}}}"#).unwrap();
    assert_eq!(cfg.settings.quick_launch.monitor, MonitorChoice::Primary);
}

#[test]
fn monitor_round_trips_through_json() {
    let mut cfg = Config::default();
    cfg.settings.quick_launch.monitor = MonitorChoice::Cursor;

    let text = serde_json::to_string(&cfg).unwrap();
    assert!(text.contains(r#""monitor":"cursor""#));

    let parsed: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed.settings.quick_launch.monitor, MonitorChoice::Cursor);
}

#[test]
fn editor_command_defaults_to_code_when_absent() {
    let cfg: Config = serde_json::from_str(r#"{"settings":{"quickLaunch":{}}}"#).unwrap();
    assert_eq!(cfg.settings.quick_launch.editor_command, "code");
}

#[test]
fn shared_settings_do_not_affect_items_or_variables() {
    let mut cfg = Config::default();
    cfg.variables
        .insert("Proj".to_string(), "D:\\work".to_string());
    let original_items_len = cfg.items.len();

    let shared_text = serde_json::to_string(&cfg.settings).unwrap();
    merge_shared_settings_text(&mut cfg, &shared_text);

    assert_eq!(cfg.items.len(), original_items_len);
    assert_eq!(cfg.variables.get("Proj").unwrap(), "D:\\work");
}
