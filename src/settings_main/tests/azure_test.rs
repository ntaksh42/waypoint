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
fn decoding_invalid_json_reports_an_error() {
    let error = decode_projects_json("not json").unwrap_err();
    assert!(error.contains("Could not parse file"));
}

#[test]
fn decoding_a_non_array_reports_an_error() {
    let error = decode_projects_json("{}").unwrap_err();
    assert!(error.contains("Could not parse file"));
}
