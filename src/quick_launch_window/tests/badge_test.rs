use super::super::badge::{AzureIconKind, azure_icon_kind};

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
