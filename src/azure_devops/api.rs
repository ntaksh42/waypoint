//! Azure DevOps REST API 呼び出し。

use std::thread;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::config::AzureDevOpsProject;

use super::Candidate;
use super::cache::{CachedRow, record_project_success, replace_project_cache};
use super::convert::{
    encode_segment, json_i64, pipeline_build_row, pipeline_definition_row, pull_request_row,
    work_item_batch_candidates, work_item_candidates,
};

pub(crate) const API_VERSION: &str = "7.1";
const PR_PAGE_SIZE: usize = 500;
const PIPELINE_BUILD_LIMIT: usize = 100;
const WORK_ITEM_RESULT_LIMIT: usize = 50;
/// `az wit` 単体で各プロジェクトから表示する最近更新 Work Item 数。
/// 監視プロジェクトが複数でも、Quick Launch のリストを過度に埋めない。
const RECENT_WORK_ITEM_LIMIT: usize = 8;
const REQUEST_RETRIES: usize = 2;
const RETRY_DELAY: Duration = Duration::from_millis(350);

pub(crate) fn refresh_project(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<(), String> {
    let mut rows = Vec::new();
    if project.include_pull_requests {
        let current_user = current_user_id(client, &project.organization, pat).ok();
        rows.extend(fetch_pull_requests(
            client,
            project,
            pat,
            current_user.as_deref(),
        )?);
    }
    if project.include_pipelines {
        rows.extend(fetch_pipelines(client, project, pat)?);
    }
    replace_project_cache(project, &rows)?;
    record_project_success(project)
}

fn fetch_pull_requests(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
) -> Result<Vec<CachedRow>, String> {
    let mut rows = Vec::new();
    let mut skip = 0;
    loop {
        let url = format!(
            "https://dev.azure.com/{}/{}/_apis/git/pullrequests?searchCriteria.status=all&$top={PR_PAGE_SIZE}&$skip={skip}&api-version={API_VERSION}",
            encode_segment(&project.organization),
            encode_segment(&project.project),
        );
        let value = get_json(client, &url, pat)?;
        let page = value["value"].as_array().cloned().unwrap_or_default();
        let count = page.len();
        rows.extend(
            page.into_iter()
                .filter_map(|item| pull_request_row(project, &item, current_user)),
        );
        if count < PR_PAGE_SIZE {
            break;
        }
        skip += PR_PAGE_SIZE;
    }
    Ok(rows)
}

pub(crate) fn current_user_id(
    client: &reqwest::blocking::Client,
    organization: &str,
    pat: &str,
) -> Result<String, String> {
    let url = format!(
        "https://dev.azure.com/{}/_apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1&api-version={API_VERSION}",
        encode_segment(organization)
    );
    get_json(client, &url, pat)?["authenticatedUser"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Azure DevOps did not return the authenticated user.".to_string())
}

fn fetch_pipelines(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<CachedRow>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let definitions = get_json(
        client,
        &format!("{base}/_apis/build/definitions?$top=1000&api-version={API_VERSION}"),
        pat,
    )?["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let builds = get_json(
        client,
        &format!("{base}/_apis/build/builds?$top={PIPELINE_BUILD_LIMIT}&api-version={API_VERSION}"),
        pat,
    )?["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut rows: Vec<_> = definitions
        .iter()
        .filter_map(|item| pipeline_definition_row(project, item))
        .collect();
    rows.extend(
        builds
            .iter()
            .filter_map(|item| pipeline_build_row(project, item)),
    );
    Ok(rows)
}

pub(crate) fn fetch_work_items(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    query: &str,
) -> Result<Vec<Candidate>, String> {
    if query.trim().is_empty() {
        return fetch_recent_work_items(client, project, pat);
    }
    let url = format!(
        "https://almsearch.dev.azure.com/{}/{}/_apis/search/workitemsearchresults?api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let value = post_json(
        client,
        &url,
        pat,
        &json!({ "searchText": query, "$skip": 0, "$top": WORK_ITEM_RESULT_LIMIT, "includeFacets": false }),
    )?;
    Ok(work_item_candidates(project, &value))
}

/// 空の `az wit` 用に、最近更新された Work Item を WIQL で絞って取得する。
/// まず ID だけを取得し、詳細は batch API で一度に読むため、プロジェクトごとの
/// 往復は二回で収まる。
pub(crate) fn fetch_recent_work_items(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<Candidate>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let query = post_json(
        client,
        &format!("{base}/_apis/wit/wiql?$top={RECENT_WORK_ITEM_LIMIT}&api-version={API_VERSION}"),
        pat,
        &json!({
            "query": "SELECT [System.Id] FROM WorkItems ORDER BY [System.ChangedDate] DESC"
        }),
    )?;
    let ids: Vec<i64> = query["workItems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| json_i64(&item["id"]))
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let items = post_json(
        client,
        &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
        pat,
        &json!({
            "ids": ids,
            "fields": ["System.Id", "System.Title", "System.State", "System.WorkItemType"],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(work_item_batch_candidates(project, &items))
}

pub(crate) fn get_json(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
) -> Result<Value, String> {
    let mut last_error = None;
    for attempt in 0..=REQUEST_RETRIES {
        match client
            .get(url)
            .header("Authorization", authorization(pat))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                return response
                    .json()
                    .map_err(|error| format!("Azure DevOps response was invalid: {error}"));
            }
            Ok(response) => {
                let status = response.status();
                last_error = Some(format!("Azure DevOps request returned HTTP {status}"));
                if !retryable_status(status.as_u16()) || attempt == REQUEST_RETRIES {
                    break;
                }
            }
            Err(error) => {
                last_error = Some(format!("Azure DevOps request failed: {error}"));
                if attempt == REQUEST_RETRIES {
                    break;
                }
            }
        }
        thread::sleep(RETRY_DELAY * (attempt as u32 + 1));
    }
    Err(last_error.unwrap_or_else(|| "Azure DevOps request failed.".to_string()))
}

pub(crate) fn post_json(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    body: &Value,
) -> Result<Value, String> {
    let mut last_error = None;
    for attempt in 0..=REQUEST_RETRIES {
        match client
            .post(url)
            .header("Authorization", authorization(pat))
            .json(body)
            .send()
        {
            Ok(response) if response.status().is_success() => {
                return response
                    .json()
                    .map_err(|error| format!("Azure DevOps response was invalid: {error}"));
            }
            Ok(response) => {
                let status = response.status();
                last_error = Some(format!("Azure DevOps request returned HTTP {status}"));
                if !retryable_status(status.as_u16()) || attempt == REQUEST_RETRIES {
                    break;
                }
            }
            Err(error) => {
                last_error = Some(format!("Azure DevOps request failed: {error}"));
                if attempt == REQUEST_RETRIES {
                    break;
                }
            }
        }
        thread::sleep(RETRY_DELAY * (attempt as u32 + 1));
    }
    Err(last_error.unwrap_or_else(|| "Azure DevOps request failed.".to_string()))
}

pub(crate) fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("Could not initialize Azure DevOps client: {error}"))
}

fn authorization(pat: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!(":{pat}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_http_statuses_are_retried() {
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }
}
