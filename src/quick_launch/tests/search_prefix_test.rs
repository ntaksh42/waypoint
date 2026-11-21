//! プレフィックス判定と、固定候補 (Web 検索 / Settings)。

use super::super::*;
use super::fixture::index;

#[test]
fn prefix_badge_identifies_each_mode() {
    assert_eq!(prefix_badge("b git"), Some("BOOKMARKS"));
    assert_eq!(prefix_badge("h waypoint"), Some("HISTORY"));
    assert_eq!(prefix_badge("az pr active waypoint"), Some("AZURE DEVOPS"));
    assert_eq!(prefix_badge("w notepad"), Some("WINDOWS"));
    assert_eq!(prefix_badge("a code"), Some("APPS"));
    assert_eq!(prefix_badge("t waypoint"), Some("TABS"));
    assert_eq!(prefix_badge("ps waypoint"), Some("TERMINAL"));
    assert_eq!(prefix_badge("ed waypoint"), Some("EDITOR"));
    assert_eq!(prefix_badge("cc E:\\waypoint review"), Some("CLAUDE CODE"));
    assert_eq!(prefix_badge("cx waypoint"), Some("CODEX"));
    assert_eq!(prefix_badge("cs waypoint"), Some("SESSIONS"));
    assert_eq!(prefix_badge("f cargo.toml"), Some("FILES"));
    assert_eq!(prefix_badge("k waypoint"), Some("KILL"));
    // `??` だけ末尾スペースを含まない (FR-9.21)
    assert_eq!(prefix_badge("?? rust lifetime"), Some("WEB"));
    assert_eq!(prefix_badge("??rust"), Some("WEB"));
    assert_eq!(prefix_badge("??"), Some("WEB"));
    assert_eq!(prefix_badge("plain query"), None);
    assert_eq!(prefix_badge(""), None);
}

#[test]
fn agent_sessions_term_drops_prefix_for_highlighting() {
    assert_eq!(effective_search_term("cs waypoint"), "waypoint");
}

#[test]
fn web_search_term_ignores_space_after_marks() {
    assert_eq!(effective_search_term("?? rust lifetime"), "rust lifetime");
    assert_eq!(effective_search_term("??rust lifetime"), "rust lifetime");
    assert_eq!(effective_search_term("??"), "");
}

/// `#` は候補名に現れないので、ハイライトには番号だけを渡す。
#[test]
fn azure_exact_id_term_drops_the_hash_for_highlighting() {
    assert_eq!(effective_search_term("az #12345"), "12345");
    assert_eq!(effective_search_term("az pr #12345"), "12345");
    assert_eq!(azure_highlight_term("#12345"), "12345");
    assert_eq!(azure_highlight_term("launcher"), "launcher");
    assert_eq!(effective_search_term("az 12345"), "12345");
    assert_eq!(effective_search_term("az #"), "#");
}

#[test]
fn claude_code_command_builds_a_single_candidate() {
    let entry = claude_code_entry("cc E:\\waypoint review42").expect("candidate missing");
    assert_eq!(entry.name, "Claude Code — review42");
    assert_eq!(entry.breadcrumb, "Start in E:\\waypoint");
    assert_eq!(entry.path, "E:\\waypoint");
    assert_eq!(
        entry.action,
        Action::OpenClaudeCode(Some("review42".into()))
    );
}

/// セッション名は省略でき、その場合は表示名なしで起動する候補になる。
#[test]
fn claude_code_command_allows_an_omitted_session_name() {
    let entry = claude_code_entry("cc E:\\waypoint ").expect("candidate missing");
    assert_eq!(entry.name, "Claude Code");
    assert_eq!(entry.action, Action::OpenClaudeCode(None));
}

#[test]
fn claude_code_command_rejects_incomplete_input() {
    for query in ["cc ", "cc E:\\waypoint"] {
        assert!(
            claude_code_entry(query).is_none(),
            "{query} should be rejected"
        );
    }
}

#[test]
fn claude_code_command_rejects_a_folder_not_selected_from_the_index() {
    let index = index();
    assert!(
        index
            .claude_code_entry(r"cc C:\not-registered review42")
            .is_none()
    );
    assert!(
        index
            .claude_code_entry(r"cc E:\waypoint\docs review42")
            .is_some()
    );
}

#[test]
fn web_search_entry_builds_single_candidate() {
    let entry = web_search_entry(crate::web_search::Engine::Google, " rust lifetime ");
    assert_eq!(entry.name, "rust lifetime");
    assert_eq!(entry.breadcrumb, "Search with Google");
    assert_eq!(
        entry.path,
        "https://www.google.com/search?q=rust%20lifetime"
    );
    assert_eq!(entry.action, Action::WebSearch);
}

#[test]
fn web_search_entry_without_term_opens_engine_home() {
    let entry = web_search_entry(crate::web_search::Engine::DuckDuckGo, "");
    assert_eq!(entry.name, "DuckDuckGo");
    assert_eq!(entry.path, "https://duckduckgo.com/");
}

#[test]
fn web_search_candidate_is_not_persisted_as_favorite() {
    // 検索語は毎回異なりうるので、お気に入り昇格の対象にしない (FR-9.21)
    let entry = web_search_entry(crate::web_search::Engine::Google, "rust");
    assert!(entry.to_item().is_none());
}

#[test]
fn settings_command_is_searchable_without_prefix() {
    // FR-9.20: 設定画面への導線は通常入力で出す。
    let index = index();
    let found = index.search("settings");
    let settings = found
        .iter()
        .find(|item| item.action == Action::OpenSettings)
        .expect("Settings candidate missing");
    assert_eq!(settings.name, "Settings");
    assert_eq!(settings.breadcrumb, "Open waypoint settings");
}

#[test]
fn settings_command_does_not_leak_into_prefix_modes() {
    // 固定候補は通常検索だけに載せる。プレフィックスモードは対象外。
    let index = index();
    for query in [
        "b settings",
        "w settings",
        "a settings",
        "ps settings",
        "ed settings",
        "cx settings",
    ] {
        assert!(
            !index
                .search(query)
                .iter()
                .any(|item| item.action == Action::OpenSettings),
            "{query} should not return the Settings command"
        );
    }
}

#[test]
fn bookmark_editor_command_is_searchable_without_prefix() {
    // FR-9.28: お気に入り編集画面への導線は通常入力で出し、プレフィックスモードには出さない。
    let index = index();
    let found = index.search("edit bookmarks");
    let editor = found
        .iter()
        .find(|item| item.action == Action::OpenBookmarkEditor)
        .expect("Edit Bookmarks candidate missing");
    assert_eq!(editor.name, "Edit Bookmarks");
    assert_eq!(editor.breadcrumb, "Edit Edge bookmarks");
    assert!(
        !index
            .search("b bookmarks")
            .iter()
            .any(|item| item.action == Action::OpenBookmarkEditor),
        "b prefix should not return the Edit Bookmarks command"
    );
}

#[test]
fn help_command_is_searchable_without_prefix() {
    // FR-9.22: Help ページへの導線は通常入力で出す。
    let index = index();
    let found = index.search("help");
    let help = found
        .iter()
        .find(|item| item.action == Action::OpenHelp)
        .expect("Help candidate missing");
    assert_eq!(help.name, "Help");
    assert_eq!(help.breadcrumb, "Open waypoint help");
}

#[test]
fn help_command_does_not_leak_into_prefix_modes() {
    // 固定候補は通常検索だけに載せる。プレフィックスモードは対象外。
    let index = index();
    for query in ["b help", "w help", "a help", "ps help", "ed help"] {
        assert!(
            !index
                .search(query)
                .iter()
                .any(|item| item.action == Action::OpenHelp),
            "{query} should not return the Help command"
        );
    }
}

#[test]
fn settings_command_is_not_persistable() {
    // FR-9.20: お気に入り昇格 (FR-9.8.1) の対象外。
    let (entries, _) = builtin_command_entries();
    assert!(entries.iter().all(|entry| entry.to_item().is_none()));
}

#[test]
fn windows_settings_prefix_matches_japanese_keywords() {
    // FR-9.29: `ws ` では検索語 (日本語の画面名) でも一致する。
    let index = Index {
        windows_settings: true,
        ..Index::default()
    };
    let found = index.search("ws 環境変数");
    assert_eq!(found[0].name, "Environment Variables");
    assert_eq!(found[0].action, Action::OpenWindowsSetting);
    assert!(
        index
            .search("ws display")
            .iter()
            .any(|e| e.path == "ms-settings:display")
    );
}

#[test]
fn windows_settings_match_names_only_without_prefix() {
    // FR-9.29: 通常入力では名前だけで照合し、別名では出さない。
    let index = Index {
        windows_settings: true,
        ..Index::default()
    };
    assert!(index.search("display").iter().any(|e| e.name == "Display"));
    assert!(
        !index
            .search("解像度")
            .iter()
            .any(|e| e.action == Action::OpenWindowsSetting)
    );
}

#[test]
fn windows_settings_disabled_hides_candidates() {
    let index = Index::default();
    assert!(index.search("ws display").is_empty());
    assert!(
        !index
            .search("display")
            .iter()
            .any(|e| e.action == Action::OpenWindowsSetting)
    );
}

#[test]
fn windows_settings_entries_are_not_persistable() {
    let (entries, _, _) = crate::quick_launch::windows_setting_entries();
    assert!(entries.iter().all(|entry| entry.to_item().is_none()));
    assert_eq!(
        crate::quick_launch::prefix_badge("ws sound"),
        Some("SETTINGS")
    );
}
