//! ホットキー文字列の解析と除外リスト判定のテスト。

use waypoint::process::is_excluded;
use waypoint::trigger::{format_hotkey, parse_hotkey};

use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

#[test]
fn parses_single_modifier() {
    let (mods, vk) = parse_hotkey("Win+W").unwrap();
    assert_eq!(mods, MOD_WIN);
    assert_eq!(vk, b'W' as u32);
}

#[test]
fn parses_multiple_modifiers() {
    let (mods, vk) = parse_hotkey("Ctrl+Alt+W").unwrap();
    assert_eq!(mods, MOD_CONTROL | MOD_ALT);
    assert_eq!(vk, b'W' as u32);
}

#[test]
fn parse_is_case_insensitive() {
    let (mods, vk) = parse_hotkey("ctrl+shift+a").unwrap();
    assert_eq!(mods, MOD_CONTROL | MOD_SHIFT);
    assert_eq!(vk, b'A' as u32);
}

#[test]
fn parses_function_key() {
    let (_, vk) = parse_hotkey("Ctrl+F5").unwrap();
    assert_eq!(vk, 0x70 + 4); // VK_F5
}

#[test]
fn parses_digit_key() {
    let (_, vk) = parse_hotkey("Alt+1").unwrap();
    assert_eq!(vk, b'1' as u32);
}

#[test]
fn rejects_empty_and_garbage() {
    assert!(parse_hotkey("").is_none());
    assert!(parse_hotkey("Ctrl+").is_none());
    // 修飾キーだけでキーが無い
    assert!(parse_hotkey("Ctrl+Alt").is_none());
    // 未対応のキー名
    assert!(parse_hotkey("Ctrl+NoSuchKey").is_none());
}

/// キー入力から設定した値 (FR-6.8.1) は、そのまま登録経路に渡る。
/// `format_hotkey` は `parse_hotkey` が読める文字列だけを返すこと。
#[test]
fn format_round_trips_through_parse() {
    for spec in [
        "Win+W",
        "Ctrl+Shift+Q",
        "Alt+1",
        "Ctrl+F5",
        "F12",
        "Alt+Space",
    ] {
        let (mods, vk) = parse_hotkey(spec).unwrap();
        assert_eq!(format_hotkey(mods, vk).as_deref(), Some(spec));
    }
}

#[test]
fn format_orders_modifiers_consistently() {
    let (_, vk) = parse_hotkey("A").unwrap();
    let all = MOD_WIN | MOD_CONTROL | MOD_ALT | MOD_SHIFT;
    assert_eq!(
        format_hotkey(all, vk).as_deref(),
        Some("Win+Ctrl+Alt+Shift+A")
    );
}

#[test]
fn format_rejects_keys_the_parser_cannot_read() {
    // VK_OEM_1 (`;`)。ホットキーには使えないので記録も拒否する
    assert!(format_hotkey(MOD_CONTROL, 0xBA).is_none());
}

#[test]
fn excluded_match_ignores_case() {
    let list = vec!["chrome.exe".to_string(), "msedge.exe".to_string()];
    assert!(is_excluded("chrome.exe", &list));
    assert!(is_excluded("CHROME.EXE", &list));
    assert!(is_excluded("Chrome.Exe", &list));
}

#[test]
fn non_excluded_process_passes() {
    let list = vec!["chrome.exe".to_string()];
    assert!(!is_excluded("explorer.exe", &list));
    assert!(!is_excluded("", &list));
}

#[test]
fn empty_exclusion_list_excludes_nothing() {
    assert!(!is_excluded("chrome.exe", &[]));
}
