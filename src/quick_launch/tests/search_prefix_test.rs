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
fn web_search_term_ignores_space_after_marks() {
    assert_eq!(effective_search_term("?? rust lifetime"), "rust lifetime");
    assert_eq!(effective_search_term("??rust lifetime"), "rust lifetime");
    assert_eq!(effective_search_term("??"), "");
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
