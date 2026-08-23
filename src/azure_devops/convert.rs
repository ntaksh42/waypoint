//! JSON レスポンスから `Candidate` / `CachedRow` への変換、および
//! URL・プロジェクトキー生成などの純粋な補助関数。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::AzureDevOpsProject;

use super::Candidate;
use super::Kind;
use super::cache::CachedRow;

pub(crate) fn project_names(value: &Value) -> Vec<String> {
    let mut names: Vec<String> = value["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|project| project["name"].as_str())
        .map(str::to_string)
        .collect();
    sort_and_deduplicate_project_names(&mut names);
    names
}

pub(crate) fn sort_and_deduplicate_project_names(names: &mut Vec<String>) {
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
}

pub(crate) fn pull_request_row(
    project: &AzureDevOpsProject,
    item: &Value,
    current_user: Option<&str>,
) -> Option<CachedRow> {
    let id = item["pullRequestId"].as_i64()?;
    let title = item["title"].as_str()?.to_string();
    let status = item["status"].as_str().unwrap_or("unknown").to_string();
    let repository = item["repository"]["name"].as_str().unwrap_or("");
    let is_mine = current_user.is_some_and(|user| {
        item["createdBy"]["id"].as_str() == Some(user)
            || item["reviewers"].as_array().is_some_and(|reviewers| {
                reviewers
                    .iter()
                    .any(|reviewer| reviewer["id"].as_str() == Some(user))
            })
    });
    let url = item["_links"]["web"]["href"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "{}/_git/{}/pullrequest/{id}",
                project_url(project),
                encode_segment(repository)
            )
        });
    Some(CachedRow {
        organization: project.organization.trim().to_string(),
        project: project.project.trim().to_string(),
        kind: "pr".to_string(),
        item_id: id.to_string(),
        status: status.clone(),
        name: format!("PR {id}: {title}"),
        detail: format!(
            "Azure DevOps — {}/{} — {}",
            project.organization, project.project, status
        ),
        url,
        is_mine,
    })
}

pub(crate) fn pipeline_definition_row(
    project: &AzureDevOpsProject,
    item: &Value,
) -> Option<CachedRow> {
    let id = item["id"].as_i64()?;
    let name = item["name"].as_str()?.to_string();
    let url = item["_links"]["web"]["href"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/_build?definitionId={id}", project_url(project)));
    Some(pipeline_row(
        project,
        format!("definition-{id}"),
        "definition",
        name,
        url,
    ))
}

pub(crate) fn pipeline_build_row(project: &AzureDevOpsProject, item: &Value) -> Option<CachedRow> {
    let id = item["id"].as_i64()?;
    let definition = item["definition"]["name"].as_str().unwrap_or("Pipeline");
    let build_number = item["buildNumber"].as_str().unwrap_or("");
    let status = item["result"]
        .as_str()
        .or_else(|| item["status"].as_str())
        .unwrap_or("unknown");
    let url = item["_links"]["web"]["href"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/_build/results?buildId={id}", project_url(project)));
    Some(pipeline_row(
        project,
        format!("build-{id}"),
        status,
        format!("{definition}: {build_number}"),
        url,
    ))
}

pub(crate) fn pipeline_row(
    project: &AzureDevOpsProject,
    item_id: String,
    status: &str,
    name: String,
    url: String,
) -> CachedRow {
    CachedRow {
        organization: project.organization.trim().to_string(),
        project: project.project.trim().to_string(),
        kind: "pipeline".to_string(),
        item_id,
        status: status.to_string(),
        detail: format!(
            "Azure DevOps — {}/{} — {status}",
            project.organization, project.project
        ),
        name,
        url,
        is_mine: false,
    }
}

pub(crate) fn work_item_candidates(project: &AzureDevOpsProject, value: &Value) -> Vec<Candidate> {
    value["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| work_item_candidate(project, &item["fields"], &item["id"]))
        .collect()
}

pub(crate) fn work_item_batch_candidates(
    project: &AzureDevOpsProject,
    value: &Value,
) -> Vec<Candidate> {
    value["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| work_item_candidate(project, &item["fields"], &item["id"]))
        .collect()
}

pub(crate) fn work_item_candidate(
    project: &AzureDevOpsProject,
    fields: &Value,
    item_id: &Value,
) -> Option<Candidate> {
    // Search API は小文字、Work Item Tracking API は PascalCase の reference
    // name を返す。どちらの取得経路でも同じ候補へ変換する。
    let id = json_i64(&fields["system.id"])
        .or_else(|| json_i64(&fields["System.Id"]))
        .or_else(|| json_i64(item_id))?;
    let title = fields["system.title"]
        .as_str()
        .or_else(|| fields["System.Title"].as_str())
        .unwrap_or("Untitled work item");
    let state = fields["system.state"]
        .as_str()
        .or_else(|| fields["System.State"].as_str())
        .unwrap_or("");
    let kind = fields["system.workitemtype"]
        .as_str()
        .or_else(|| fields["System.WorkItemType"].as_str())
        .unwrap_or("Work Item");
    Some(Candidate {
        kind: Kind::WorkItem,
        status: state.to_string(),
        name: format!("{id}: {title}"),
        detail: format!(
            "Azure DevOps — {}/{} — {kind} {state}",
            project.organization, project.project
        ),
        url: format!("{}/_workitems/edit/{id}", project_url(project)),
        organization: project.organization.trim().to_string(),
        project: project.project.trim().to_string(),
        aliases: project.aliases.clone(),
        priority: project.priority,
        is_mine: false,
    })
}

/// Search API は `System.Id` を数値または文字列で返す。どちらでも候補を
/// 捨てないよう、数値化をここへ閉じ込める。
pub(crate) fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn project_url(project: &AzureDevOpsProject) -> String {
    format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(project.organization.trim()),
        encode_segment(project.project.trim())
    )
}

pub(crate) fn project_key(organization: &str, project: &str) -> String {
    format!(
        "{}/{}",
        organization.trim().to_lowercase(),
        project.trim().to_lowercase()
    )
}

pub(crate) fn valid_project(project: &AzureDevOpsProject) -> bool {
    !project.organization.trim().is_empty() && !project.project.trim().is_empty()
}

pub(crate) fn encode_segment(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                format!("{}", byte as char).chars().collect::<Vec<_>>()
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn project_names_sort_and_deduplicate_case_insensitively() {
        assert_eq!(
            project_names(&json!({ "value": [
                { "name": "zebra" }, { "name": "Alpha" }, { "name": "alpha" }
            ]})),
            ["Alpha", "zebra"]
        );
    }
}
