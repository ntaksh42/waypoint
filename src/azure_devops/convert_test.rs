//! `convert.rs` のテスト。本体が 500 行規約を超えたため分離した
//! (`candidate_cache.rs` と同じ、ファイル全体がテストモジュールの形)。

#[cfg(test)]
mod tests {
    use crate::azure_devops::convert::*;
    use crate::config::AzureDevOpsProject;
    use serde_json::json;

    #[test]
    fn work_item_results_use_the_web_work_item_url() {
        let project = AzureDevOpsProject {
            organization: "org".to_string(),
            project: "Project Name".to_string(),
            aliases: vec!["p".to_string()],
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        };
        let results = work_item_candidates(
            &project,
            &json!({ "results": [{ "fields": {
                "system.id": "42",
                "system.title": "Fix launcher",
                "system.state": "Active",
                "system.workitemtype": "Bug"
            }}]}),
        );
        assert_eq!(results[0].name, "42: Fix launcher");
        assert_eq!(
            results[0].url,
            "https://dev.azure.com/org/Project%20Name/_workitems/edit/42"
        );
    }

    #[test]
    fn work_item_results_accept_string_ids_from_search() {
        let project = AzureDevOpsProject {
            organization: "org".to_string(),
            project: "project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        };
        let results = work_item_candidates(
            &project,
            &json!({ "results": [{ "fields": {
                "system.id": "73", "system.title": "Fix WIT search"
            }}]}),
        );
        assert_eq!(results[0].name, "73: Fix WIT search");
    }

    #[test]
    fn recent_work_items_accept_batch_api_fields() {
        let project = AzureDevOpsProject {
            organization: "org".to_string(),
            project: "project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        };
        let results = work_item_batch_candidates(
            &project,
            &json!({ "value": [{ "id": 91, "fields": {
                "System.Title": "Recent bug",
                "System.State": "Active",
                "System.WorkItemType": "Bug"
            }}]}),
        );
        assert_eq!(results[0].name, "91: Recent bug");
        assert_eq!(results[0].status, "Active");
    }

    fn pr_project() -> AzureDevOpsProject {
        AzureDevOpsProject {
            organization: "org".to_string(),
            project: "project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        }
    }

    /// `az pr needs-review` / `waiting` / `ready` / `draft` のライブ検索は
    /// この導出フラグだけを見る。生 JSON から立てられないと、絞り込みが
    /// 効かないまま Active PR が丸ごと返る。
    #[test]
    fn pull_request_row_derives_the_review_state_filters_from_reviewer_votes() {
        let row = pull_request_row(
            &pr_project(),
            &json!({
                "pullRequestId": 7,
                "title": "Add az filters",
                "status": "active",
                "isDraft": true,
                "createdBy": { "id": "me", "displayName": "Me" },
                "reviewers": [
                    { "id": "me", "vote": 0, "isRequired": true },
                    { "id": "other", "vote": -5, "isRequired": true }
                ]
            }),
            Some("me"),
        )
        .expect("row");
        assert!(row.is_author);
        assert!(row.is_reviewer);
        assert!(row.is_draft);
        assert!(row.needs_my_review, "未投票の reviewer なので要レビュー");
        assert!(row.waiting_for_others, "required に未投票・差し戻しが残る");
        assert!(!row.ready_to_complete);
    }

    #[test]
    fn pull_request_row_marks_ready_to_complete_when_every_required_reviewer_approved() {
        let row = pull_request_row(
            &pr_project(),
            &json!({
                "pullRequestId": 8,
                "title": "Ready",
                "status": "active",
                "createdBy": { "id": "me" },
                "reviewers": [{ "id": "other", "vote": 10, "isRequired": true }]
            }),
            Some("me"),
        )
        .expect("row");
        assert!(row.ready_to_complete);
        assert!(!row.waiting_for_others);
        assert!(!row.needs_my_review, "自分は reviewer ではない");
    }

    /// Completed / Abandoned に Active 前提のフラグを立てない。
    #[test]
    fn closed_pull_requests_never_carry_the_active_only_filters() {
        let row = pull_request_row(
            &pr_project(),
            &json!({
                "pullRequestId": 9,
                "title": "Old",
                "status": "completed",
                "creationDate": "2000-01-01T00:00:00Z",
                "createdBy": { "id": "me" },
                "reviewers": [{ "id": "me", "vote": 0, "isRequired": true }]
            }),
            Some("me"),
        )
        .expect("row");
        assert!(!row.needs_my_review);
        assert!(!row.waiting_for_others);
        assert!(!row.ready_to_complete);
        assert!(!row.is_stale);
    }

    #[test]
    fn active_pull_requests_older_than_two_weeks_are_stale() {
        let row = pull_request_row(
            &pr_project(),
            &json!({
                "pullRequestId": 10,
                "title": "Stale",
                "status": "active",
                "creationDate": "2000-01-01T00:00:00Z"
            }),
            None,
        )
        .expect("row");
        assert!(row.is_stale);
    }

    /// 自分の ID が未解決でも他人の PR を `author` にしない。
    #[test]
    fn unknown_identity_never_claims_authorship() {
        let row = pull_request_row(
            &pr_project(),
            &json!({
                "pullRequestId": 11,
                "title": "Someone else",
                "status": "active",
                "createdBy": { "displayName": "Other" }
            }),
            None,
        )
        .expect("row");
        assert!(!row.is_author);
        assert!(!row.is_mine);
    }

    #[test]
    fn project_names_sort_and_deduplicate_case_insensitively() {
        assert_eq!(
            project_names(&json!({ "value": [
                { "name": "zebra" }, { "name": "Alpha" }, { "name": "alpha" }
            ]})),
            ["Alpha", "zebra"]
        );
    }

    #[test]
    fn repository_names_and_ids_extracts_pairs_sorted_by_name() {
        let response = json!({ "value": [
            { "name": "zebra-repo", "id": "id-z" },
            { "name": "alpha-repo", "id": "id-a" },
        ]});
        assert_eq!(
            repository_names_and_ids(&response),
            vec![
                ("alpha-repo".to_string(), "id-a".to_string()),
                ("zebra-repo".to_string(), "id-z".to_string()),
            ]
        );
    }

    #[test]
    fn repository_names_and_ids_skips_entries_missing_name_or_id() {
        let response = json!({ "value": [
            { "name": "only-name" },
            { "id": "only-id" },
            { "name": "complete", "id": "id-c" },
        ]});
        assert_eq!(
            repository_names_and_ids(&response),
            vec![("complete".to_string(), "id-c".to_string())]
        );
    }

    #[test]
    fn area_nodes_flattens_nested_children_with_depth_and_full_path() {
        let response = json!({
            "name": "Waypoint",
            "children": [
                {
                    "name": "Team",
                    "children": [
                        { "name": "Launcher" },
                        { "name": "Search" }
                    ]
                },
                { "name": "Settings UI" }
            ]
        });
        let nodes = area_nodes(&response);
        assert_eq!(
            nodes,
            vec![
                AreaNode {
                    path: "Waypoint\\Team".to_string(),
                    depth: 0
                },
                AreaNode {
                    path: "Waypoint\\Team\\Launcher".to_string(),
                    depth: 1
                },
                AreaNode {
                    path: "Waypoint\\Team\\Search".to_string(),
                    depth: 1
                },
                AreaNode {
                    path: "Waypoint\\Settings UI".to_string(),
                    depth: 0
                },
            ]
        );
    }

    #[test]
    fn area_nodes_is_empty_without_children() {
        assert!(area_nodes(&json!({ "name": "Waypoint" })).is_empty());
    }

    #[test]
    fn area_path_counts_orders_by_frequency_then_name() {
        let response = json!({ "value": [
            { "fields": { "System.AreaPath": "Waypoint\\Search" } },
            { "fields": { "System.AreaPath": "Waypoint\\Launcher" } },
            { "fields": { "System.AreaPath": "Waypoint\\Launcher" } },
            { "fields": { "System.AreaPath": "Waypoint\\Settings UI" } },
        ]});
        assert_eq!(
            area_path_counts(&response),
            vec![
                ("Waypoint\\Launcher".to_string(), 2),
                ("Waypoint\\Search".to_string(), 1),
                ("Waypoint\\Settings UI".to_string(), 1),
            ]
        );
    }

    #[test]
    fn area_path_counts_ignores_items_without_an_area_path() {
        let response = json!({ "value": [
            { "fields": {} },
            { "fields": { "System.AreaPath": "Waypoint" } },
        ]});
        assert_eq!(
            area_path_counts(&response),
            vec![("Waypoint".to_string(), 1)]
        );
    }

    #[test]
    fn area_path_counts_is_empty_without_values() {
        assert!(area_path_counts(&json!({})).is_empty());
    }

    #[test]
    fn parse_rfc3339_unix_matches_known_epoch_values() {
        assert_eq!(parse_rfc3339_unix("1970-01-01T00:00:00Z"), Some(0));
        // date -u -d "2024-01-15T10:30:00Z" +%s
        assert_eq!(
            parse_rfc3339_unix("2024-01-15T10:30:00.123Z"),
            Some(1_705_314_600)
        );
        // うるう年 (2024) の 2/29 をまたぐ日付も正しく積算できること
        assert_eq!(
            parse_rfc3339_unix("2024-03-01T00:00:00Z"),
            Some(1_709_251_200)
        );
    }

    #[test]
    fn parse_rfc3339_unix_rejects_malformed_input() {
        assert_eq!(parse_rfc3339_unix(""), None);
        assert_eq!(parse_rfc3339_unix("not-a-date"), None);
        assert_eq!(parse_rfc3339_unix("2024/01/15T10:30:00Z"), None);
    }
}
