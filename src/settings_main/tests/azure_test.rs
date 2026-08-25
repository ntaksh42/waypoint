use waypoint::azure_devops::ProjectActivity;
use waypoint::config::AzureDevOpsProject;

use super::super::azure_draft::{
    AzureProjectPicker, Scope, decode_projects_json, encode_projects_json,
};

fn project(organization: &str, project: &str) -> AzureDevOpsProject {
    AzureDevOpsProject {
        organization: organization.to_string(),
        project: project.to_string(),
        aliases: Vec::new(),
        priority: 0,
        include_pull_requests: true,
        include_pipelines: true,
        include_work_items: true,
        interest_areas: Vec::new(),
        interest_repositories: Vec::new(),
    }
}

fn picker_with(projects: Vec<AzureDevOpsProject>, loaded_organization: &str) -> AzureProjectPicker {
    let mut picker = AzureProjectPicker::new(projects);
    picker.loaded_organization = loaded_organization.to_string();
    picker
}

#[test]
fn checking_a_project_adds_it_with_default_scopes() {
    let mut picker = picker_with(Vec::new(), "contoso");
    picker.set_selected("Waypoint", true);
    assert!(picker.is_selected("contoso", "Waypoint"));
    let entry = picker
        .projects
        .iter()
        .find(|entry| entry.project == "Waypoint")
        .unwrap();
    assert!(entry.include_pull_requests);
    assert!(entry.include_pipelines);
    assert!(entry.include_work_items);
}

#[test]
fn unchecking_a_project_removes_it_and_closes_its_detail_panel() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");
    assert!(picker.selected.is_some());

    picker.set_selected("Waypoint", false);

    assert!(!picker.is_selected("contoso", "Waypoint"));
    assert!(picker.selected.is_none());
}

#[test]
fn unchecking_one_project_keeps_other_organizations_untouched() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("fabrikam", "Platform"),
        ],
        "contoso",
    );
    picker.set_selected("Waypoint", false);
    assert_eq!(picker.projects.len(), 1);
    assert_eq!(picker.projects[0].project, "Platform");
}

#[test]
fn commit_text_fields_parses_aliases_and_priority_into_the_selected_project() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");
    picker.aliases_text = " wp, launcher ,, ".to_string();
    picker.priority_text = "10".to_string();

    picker.commit_text_fields();

    let entry = &picker.projects[0];
    assert_eq!(entry.aliases, ["wp", "launcher"]);
    assert_eq!(entry.priority, 10);
}

#[test]
fn commit_text_fields_treats_invalid_priority_as_zero() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");
    picker.priority_text = "not a number".to_string();

    picker.commit_text_fields();

    assert_eq!(picker.projects[0].priority, 0);
}

#[test]
fn switching_the_open_project_commits_the_previous_edits_first() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("contoso", "Platform"),
        ],
        "contoso",
    );
    picker.open_detail("contoso", "Waypoint");
    picker.aliases_text = "wp".to_string();

    picker.open_detail("contoso", "Platform");

    let waypoint = picker
        .projects
        .iter()
        .find(|entry| entry.project == "Waypoint")
        .unwrap();
    assert_eq!(waypoint.aliases, ["wp"]);
}

#[test]
fn set_scope_toggles_only_the_targeted_scope() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.set_scope("contoso", "Waypoint", Scope::Pipelines, false);
    let entry = &picker.projects[0];
    assert!(entry.include_pull_requests);
    assert!(!entry.include_pipelines);
    assert!(entry.include_work_items);
}

#[test]
fn toggle_interest_area_adds_and_removes_paths_on_the_selected_project() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");

    picker.toggle_interest_area("Waypoint\\Launcher", true);
    picker.toggle_interest_area("Waypoint\\Search", true);
    assert_eq!(
        picker.selected_interest_areas(),
        [
            "Waypoint\\Launcher".to_string(),
            "Waypoint\\Search".to_string()
        ]
        .into_iter()
        .collect()
    );

    picker.toggle_interest_area("Waypoint\\Launcher", false);
    assert_eq!(
        picker.selected_interest_areas(),
        ["Waypoint\\Search".to_string()].into_iter().collect()
    );
}

#[test]
fn toggling_multiple_filtered_paths_accumulates_and_can_be_cleared_together() {
    // Select shown / Clear shown はフィルタ後のノード集合を順に toggle するだけなので、
    // ここでは複数パスへの一括トグルが正しく積み上がる/消えることを確認する。
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");
    let shown = [
        "Waypoint\\Launcher",
        "Waypoint\\Search",
        "Waypoint\\Settings UI",
    ];

    for path in shown {
        picker.toggle_interest_area(path, true);
    }
    assert_eq!(picker.selected_interest_areas().len(), 3);

    for path in shown {
        picker.toggle_interest_area(path, false);
    }
    assert!(picker.selected_interest_areas().is_empty());
}

#[test]
fn clear_interest_areas_empties_only_the_selected_project() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("contoso", "Platform"),
        ],
        "contoso",
    );
    picker.open_detail("contoso", "Waypoint");
    picker.toggle_interest_area("Waypoint\\Launcher", true);
    picker.open_detail("contoso", "Platform");
    picker.toggle_interest_area("Platform\\Core", true);

    picker.open_detail("contoso", "Waypoint");
    picker.clear_interest_areas();

    assert!(picker.selected_interest_areas().is_empty());
    picker.open_detail("contoso", "Platform");
    assert_eq!(
        picker.selected_interest_areas(),
        ["Platform\\Core".to_string()].into_iter().collect()
    );
}

#[test]
fn toggle_interest_repository_adds_and_removes_names_on_the_selected_project() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.open_detail("contoso", "Waypoint");

    picker.toggle_interest_repository("launcher-repo", true);
    picker.toggle_interest_repository("search-repo", true);
    assert_eq!(
        picker.selected_interest_repositories(),
        ["launcher-repo".to_string(), "search-repo".to_string()]
            .into_iter()
            .collect()
    );

    picker.toggle_interest_repository("launcher-repo", false);
    assert_eq!(
        picker.selected_interest_repositories(),
        ["search-repo".to_string()].into_iter().collect()
    );
}

#[test]
fn clear_interest_repositories_empties_only_the_selected_project() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("contoso", "Platform"),
        ],
        "contoso",
    );
    picker.open_detail("contoso", "Waypoint");
    picker.toggle_interest_repository("launcher-repo", true);
    picker.open_detail("contoso", "Platform");
    picker.toggle_interest_repository("core-repo", true);

    picker.open_detail("contoso", "Waypoint");
    picker.clear_interest_repositories();

    assert!(picker.selected_interest_repositories().is_empty());
    picker.open_detail("contoso", "Platform");
    assert_eq!(
        picker.selected_interest_repositories(),
        ["core-repo".to_string()].into_iter().collect()
    );
}

fn project_with_details(organization: &str, project: &str) -> AzureDevOpsProject {
    AzureDevOpsProject {
        organization: organization.to_string(),
        project: project.to_string(),
        aliases: vec!["wp".to_string()],
        priority: 5,
        include_pull_requests: true,
        include_pipelines: false,
        include_work_items: true,
        interest_areas: vec!["Waypoint\\Launcher".to_string()],
        interest_repositories: Vec::new(),
    }
}

#[test]
fn projects_round_trip_through_json() {
    let projects = vec![
        project_with_details("contoso", "Waypoint"),
        project_with_details("contoso", "Platform"),
    ];
    let json = encode_projects_json(&projects).unwrap();
    let decoded = decode_projects_json(&json).unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].organization, "contoso");
    assert_eq!(decoded[0].aliases, ["wp"]);
    assert_eq!(decoded[0].interest_areas, ["Waypoint\\Launcher"]);
    assert!(!decoded[0].include_pipelines);
}

#[test]
fn decoding_a_project_without_interest_repositories_defaults_to_empty() {
    // interest_repositories 追加前にエクスポートされた JSON でも読み込める
    // ことを確認する (`#[serde(default)]` によるフィールド追加の後方互換性)。
    let json = r#"[{
        "organization": "contoso",
        "project": "Waypoint",
        "aliases": [],
        "priority": 0,
        "includePullRequests": true,
        "includePipelines": true,
        "includeWorkItems": true,
        "interestAreas": []
    }]"#;
    let decoded = decode_projects_json(json).unwrap();
    assert_eq!(decoded.len(), 1);
    assert!(decoded[0].interest_repositories.is_empty());
}

#[test]
fn decoding_invalid_json_reports_an_error() {
    let error = decode_projects_json("not json").unwrap_err();
    assert!(error.contains("Could not parse file"));
}

#[test]
fn decoding_a_non_array_reports_an_error() {
    let error = decode_projects_json("{}").unwrap_err();
    assert!(error.contains("Could not parse file"));
}

fn activity(
    organization: &str,
    project: &str,
    count: usize,
    areas: Vec<(&str, usize)>,
) -> ProjectActivity {
    ProjectActivity {
        organization: organization.to_string(),
        project: project.to_string(),
        count,
        areas: areas
            .into_iter()
            .map(|(path, n)| (path.to_string(), n))
            .collect(),
    }
}

#[test]
fn applying_checked_suggestions_ranks_priority_by_list_order() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("contoso", "Platform"),
        ],
        "contoso",
    );
    picker.priority_suggestions = vec![
        activity("contoso", "Waypoint", 10, vec![("Waypoint\\Launcher", 7)]),
        activity("contoso", "Platform", 3, vec![("Platform\\Core", 2)]),
    ];
    picker
        .priority_suggestion_checked
        .insert(("contoso".to_string(), "Waypoint".to_string()));
    picker
        .priority_suggestion_checked
        .insert(("contoso".to_string(), "Platform".to_string()));

    picker.apply_priority_suggestions();

    let waypoint = picker
        .projects
        .iter()
        .find(|entry| entry.project == "Waypoint")
        .unwrap();
    let platform = picker
        .projects
        .iter()
        .find(|entry| entry.project == "Platform")
        .unwrap();
    assert_eq!(waypoint.priority, 0);
    assert_eq!(platform.priority, 1);
    assert!(!picker.priority_suggestion_open);
}

#[test]
fn applying_suggestions_ignores_unchecked_projects() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");
    picker.projects[0].priority = 9;
    picker.priority_suggestions = vec![activity(
        "contoso",
        "Waypoint",
        5,
        vec![("Waypoint\\Launcher", 5)],
    )];
    // 何もチェックしていない状態で Apply しても書き換わらないこと。

    picker.apply_priority_suggestions();

    assert_eq!(picker.projects[0].priority, 9);
}

#[test]
fn toggling_a_priority_suggestion_area_updates_that_projects_interest_areas() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");

    picker.toggle_priority_suggestion_area("contoso", "Waypoint", "Waypoint\\Launcher", true);
    assert_eq!(
        picker.projects[0].interest_areas,
        ["Waypoint\\Launcher".to_string()]
    );

    picker.toggle_priority_suggestion_area("contoso", "Waypoint", "Waypoint\\Launcher", false);
    assert!(picker.projects[0].interest_areas.is_empty());
}

#[test]
fn toggling_a_priority_suggestion_area_does_not_affect_other_projects() {
    let mut picker = picker_with(
        vec![
            project("contoso", "Waypoint"),
            project("contoso", "Platform"),
        ],
        "contoso",
    );

    picker.toggle_priority_suggestion_area("contoso", "Waypoint", "Waypoint\\Launcher", true);

    let platform = picker
        .projects
        .iter()
        .find(|entry| entry.project == "Platform")
        .unwrap();
    assert!(platform.interest_areas.is_empty());
}

#[test]
fn expanding_a_suggestion_row_toggles_and_collapses_on_second_call() {
    let mut picker = picker_with(vec![project("contoso", "Waypoint")], "contoso");

    picker.toggle_priority_suggestion_expanded("contoso", "Waypoint");
    assert_eq!(
        picker.priority_suggestion_expanded,
        Some(("contoso".to_string(), "Waypoint".to_string()))
    );

    picker.toggle_priority_suggestion_expanded("contoso", "Waypoint");
    assert_eq!(picker.priority_suggestion_expanded, None);
}

#[test]
fn toggling_a_priority_suggestion_area_on_twice_does_not_duplicate_it() {
    // project_with_details は既に "Waypoint\Launcher" を interest_areas に持つ
    let mut picker = picker_with(vec![project_with_details("contoso", "Waypoint")], "contoso");

    picker.toggle_priority_suggestion_area("contoso", "Waypoint", "Waypoint\\Launcher", true);

    let areas = &picker.projects[0].interest_areas;
    assert_eq!(
        areas.iter().filter(|a| *a == "Waypoint\\Launcher").count(),
        1
    );
}
