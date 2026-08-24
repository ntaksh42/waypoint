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

/// 1 つの Area ノード。`depth` は描画時のインデントに使う (0 がプロジェクト直下)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaNode {
    pub path: String,
    pub depth: usize,
}

/// `_apis/wit/classificationnodes/Areas` のレスポンスを深さ優先で平坦化する。
/// ルート (プロジェクト名) 自体はノードに含めない。
pub(crate) fn area_nodes(value: &Value) -> Vec<AreaNode> {
    let mut nodes = Vec::new();
    if let Some(children) = value["children"].as_array() {
        let root = value["name"].as_str().unwrap_or_default();
        for child in children {
            collect_area_node(child, root, 0, &mut nodes);
        }
    }
    nodes
}

fn collect_area_node(value: &Value, parent_path: &str, depth: usize, out: &mut Vec<AreaNode>) {
    let Some(name) = value["name"].as_str() else {
        return;
    };
    let path = if parent_path.is_empty() {
        name.to_string()
    } else {
        format!("{parent_path}\\{name}")
    };
    out.push(AreaNode {
        path: path.clone(),
        depth,
    });
    if let Some(children) = value["children"].as_array() {
        for child in children {
            collect_area_node(child, &path, depth + 1, out);
        }
    }
}

/// `workitemsbatch` のレスポンスから `System.AreaPath` を件数の多い順に集計する。
/// 自分の担当 Work Item から興味のある Area Path を推薦する材料に使う。
pub(crate) fn area_path_counts(value: &Value) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in value["value"].as_array().into_iter().flatten() {
        let area = item["fields"]["System.AreaPath"]
            .as_str()
            .or_else(|| item["fields"]["system.areapath"].as_str());
        if let Some(area) = area {
            *counts.entry(area.to_string()).or_insert(0) += 1;
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
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
    let author = item["createdBy"]["displayName"].as_str().unwrap_or("");
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
        detail: if author.is_empty() {
            format!(
                "Azure DevOps — {}/{} — {}",
                project.organization, project.project, status
            )
        } else {
            format!(
                "Azure DevOps — {}/{} — {} — by {author}",
                project.organization, project.project, status
            )
        },
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

/// Azure DevOps が返す `creationDate` (`2024-01-15T10:30:00.123Z` 形式) を
/// unix 秒へ変換する。日付部分だけをうるう年込みで積算し、外部クレートを
/// 追加せずに済ませる。パース出来ない値は `None` を返し、呼び出し側は
/// 「打ち切り対象にしない」側へフォールバックする。
pub(crate) fn parse_rfc3339_unix(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    let hour: i64 = text.get(11..13)?.parse().ok()?;
    let minute: i64 = text.get(14..16)?.parse().ok()?;
    let second: i64 = text.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let days_since_epoch = days_from_civil(year, month, day);
    Some(days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Howard Hinnant の `days_from_civil` アルゴリズム。1970-01-01 を 0 とする
/// 通算日数をグレゴリオ暦の年月日から求める (うるう年を含めて厳密)。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
            interest_areas: Vec::new(),
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
