//! 補完と結果の併記、および Repository の厳密な限定。

use super::super::search::LowerKeys;
use super::fixture::index;

#[test]
fn azure_completion_keeps_matching_pull_requests() {
    let mut index = index();
    index.azure[0].entry.name = "PR 42: pipeline".into();
    index.azure[0].lower = LowerKeys::new(&index.azure[0].entry);
    let names: Vec<_> = index
        .search("az pln")
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert!(names.contains(&"az pipeline"));
    assert!(names.contains(&"PR 42: pipeline"));
}

#[test]
fn azure_repository_scope_excludes_same_names_in_other_projects_and_organizations() {
    let mut index = index();
    let base = index.azure[0].clone();
    for url in [
        "https://dev.azure.com/other/Waypoint/_git/app/pullrequest/43",
        "https://dev.azure.com/org/Other/_git/app/pullrequest/44",
        "https://dev.azure.com/org/Waypoint/_git/app-tools/pullrequest/45",
    ] {
        let mut item = base.clone();
        item.entry.path = url.into();
        item.lower = LowerKeys::new(&item.entry);
        index.azure.push(item);
    }
    let found = index.search("az pr needs-review repo:org/Waypoint/app search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, base.entry.path);
    assert_eq!(
        super::super::effective_search_term("az pr repo:org/Waypoint/app #42"),
        "42"
    );
    assert_eq!(index.search("az pr repo:org/Waypoint/app #42").len(), 1);
}
