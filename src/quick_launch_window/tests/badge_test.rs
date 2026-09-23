use super::super::RowKind;
use super::super::badge::{
    AzureIconKind, action_tag, action_verb, azure_icon_kind, shows_live_search_hint,
};
use super::super::draw_row::section_count;
use crate::quick_launch::Action;

#[test]
fn live_search_hint_is_only_shown_in_azure_mode() {
    assert!(shows_live_search_hint(Some("AZURE DEVOPS")));
    assert!(!shows_live_search_hint(Some("FILES")));
    assert!(!shows_live_search_hint(None));
}

#[test]
fn azure_urls_use_distinct_icons_only_in_azure_mode() {
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_git/repo/pullrequest/42"
        ),
        Some(AzureIconKind::PullRequest)
    );
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_workitems/edit/91"
        ),
        Some(AzureIconKind::WorkItem)
    );
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_build/results?buildId=8"
        ),
        Some(AzureIconKind::Pipeline)
    );
    assert_eq!(
        azure_icon_kind(Some("AZURE DEVOPS"), "https://dev.azure.com/org/project"),
        Some(AzureIconKind::Project)
    );
    assert_eq!(
        azure_icon_kind(Some("BOOKMARKS"), "https://dev.azure.com/org/project"),
        None
    );
}

#[test]
fn action_tags_name_the_candidate_kind() {
    assert_eq!(action_tag(&Action::LaunchApp, None, r"C:\x.lnk"), "App");
    assert_eq!(action_tag(&Action::FocusWindow(1), None, ""), "Window");
    assert_eq!(
        action_tag(&Action::KillProcess(4), Some("KILL"), ""),
        "Kill"
    );
    // `cc ` のフォルダ候補 (path を持つ ReplaceQuery) はフォルダとして見せる
    assert_eq!(
        action_tag(&Action::ReplaceQuery("cc x".into()), None, r"E:\x"),
        "Folder"
    );
    assert_eq!(
        action_tag(&Action::ReplaceQuery("az pr ".into()), None, ""),
        "Command"
    );
}

#[test]
fn azure_candidates_are_tagged_by_url_only_in_azure_mode() {
    let url = "https://dev.azure.com/org/project/_git/repo/pullrequest/42";
    let action = Action::OpenUrl(url.into());
    assert_eq!(action_tag(&action, Some("AZURE DEVOPS"), url), "PR");
    assert_eq!(action_tag(&action, Some("BOOKMARKS"), url), "Link");
}

#[test]
fn selected_row_verbs_describe_what_enter_does() {
    assert_eq!(action_verb(&Action::FocusWindow(1)), "Switch \u{21B5}");
    assert_eq!(action_verb(&Action::LaunchApp), "Launch \u{21B5}");
    assert_eq!(action_verb(&Action::KillProcess(4)), "Kill \u{21B5}");
    assert_eq!(action_verb(&Action::OpenHelp), "Open \u{21B5}");
}

#[test]
fn section_count_stops_at_the_next_header() {
    let rows = [
        RowKind::Header("Folders"),
        RowKind::Item(0),
        RowKind::Item(1),
        RowKind::Header("Apps"),
        RowKind::Item(2),
    ];
    assert_eq!(section_count(&rows, 0), 2);
    assert_eq!(section_count(&rows, 3), 1);
    assert_eq!(section_count(&[RowKind::Header("Folders")], 0), 0);
}
