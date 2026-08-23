use super::super::trigger_draft::{merge_selected_azure_projects, parse_azure_projects};

#[test]
fn parses_multiple_azure_devops_projects_with_aliases_and_priority() {
    let projects =
        parse_azure_projects("contoso/Waypoint | wp, launcher | 10\ncontoso/Platform | infra | 20")
            .unwrap();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].organization, "contoso");
    assert_eq!(projects[0].aliases, ["wp", "launcher"]);
    assert_eq!(projects[1].priority, 20);
}

#[test]
fn rejects_duplicate_azure_devops_projects() {
    let error = parse_azure_projects("contoso/Waypoint\nCONTOSO/waypoint")
        .expect_err("duplicate project should fail");
    assert!(error.contains("duplicates"));
}

#[test]
fn applying_checked_azure_projects_keeps_other_organizations_and_metadata() {
    let watched_projects =
        "contoso/Waypoint | wp | 10\nfabrikam/Platform | platform | 20".to_string();
    let selected = ["New Project".to_string(), "Waypoint".to_string()]
        .into_iter()
        .collect();

    let updated = merge_selected_azure_projects(&watched_projects, "contoso", &selected).unwrap();

    let projects = parse_azure_projects(&updated).unwrap();
    assert_eq!(projects.len(), 3);
    assert_eq!(projects[0].organization, "fabrikam");
    assert_eq!(projects[1].project, "New Project");
    assert_eq!(projects[2].aliases, ["wp"]);
    assert_eq!(projects[2].priority, 10);
}

#[test]
fn parses_per_project_azure_sync_scopes() {
    let projects = parse_azure_projects("contoso/Waypoint | wp | 10 | pr,wit").unwrap();
    assert!(projects[0].include_pull_requests);
    assert!(!projects[0].include_pipelines);
    assert!(projects[0].include_work_items);
}
