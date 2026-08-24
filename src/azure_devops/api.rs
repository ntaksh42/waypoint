//! Azure DevOps REST API 呼び出し。

use std::thread;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::config::AzureDevOpsProject;

use super::Candidate;
use super::cache::{CachedRow, record_project_success, replace_project_cache};
use super::convert::{
    AreaNode, area_nodes, area_path_counts, encode_segment, json_i64, parse_rfc3339_unix,
    pipeline_build_row, pipeline_definition_row, pull_request_row, unix_timestamp,
    work_item_batch_candidates, work_item_candidates,
};

pub(crate) const API_VERSION: &str = "7.1";
const PR_PAGE_SIZE: usize = 500;
/// Completed / Abandoned PR はこれより古い作成日のページに達したら打ち切る。
/// Active は状態そのものが「今関心がある」ことの表明なので上限を設けない。
const PR_HISTORY_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 90);
/// 上の日付条件に加えた保険。1 プロジェクトが Completed/Abandoned だけで
/// 何千件もページングし続けないよう件数でも打ち切る。
const PR_HISTORY_MAX_COUNT: usize = 1_000;
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

/// Active PR は無条件で全件、Completed / Abandoned は新しい順に読み、
/// 作成日が `PR_HISTORY_MAX_AGE` を超えるか `PR_HISTORY_MAX_COUNT` に
/// 達したら打ち切る。プロジェクトの累積 PR 数が数十万件規模でも、
/// 同期が終わらなくなるのを防ぐ (R-13)。
fn fetch_pull_requests(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
) -> Result<Vec<CachedRow>, String> {
    let mut rows = fetch_pull_requests_by_status(client, project, pat, current_user, "active")?;
    rows.extend(fetch_pull_requests_history(
        client,
        project,
        pat,
        current_user,
        "completed",
    )?);
    rows.extend(fetch_pull_requests_history(
        client,
        project,
        pat,
        current_user,
        "abandoned",
    )?);
    Ok(rows)
}

fn fetch_pull_requests_by_status(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    status: &str,
) -> Result<Vec<CachedRow>, String> {
    let mut rows = Vec::new();
    let mut skip = 0;
    loop {
        let url = format!(
            "https://dev.azure.com/{}/{}/_apis/git/pullrequests?searchCriteria.status={status}&$top={PR_PAGE_SIZE}&$skip={skip}&api-version={API_VERSION}",
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

/// Completed / Abandoned 用。ページは `creationDate` 降順で返るため、
/// ページ内最古の作成日が上限を超えたらそこで打ち切ってよい。
fn fetch_pull_requests_history(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    status: &str,
) -> Result<Vec<CachedRow>, String> {
    let cutoff = unix_timestamp().saturating_sub(PR_HISTORY_MAX_AGE.as_secs() as i64);
    let mut rows = Vec::new();
    let mut skip = 0;
    loop {
        let url = format!(
            "https://dev.azure.com/{}/{}/_apis/git/pullrequests?searchCriteria.status={status}&$top={PR_PAGE_SIZE}&$skip={skip}&api-version={API_VERSION}",
            encode_segment(&project.organization),
            encode_segment(&project.project),
        );
        let value = get_json(client, &url, pat)?;
        let page = value["value"].as_array().cloned().unwrap_or_default();
        let count = page.len();
        let oldest_in_page = page.iter().filter_map(creation_date_unix).min();
        rows.extend(
            page.into_iter()
                .filter(|item| {
                    creation_date_unix(item).is_none_or(|created| created >= cutoff)
                })
                .filter_map(|item| pull_request_row(project, &item, current_user)),
        );
        if should_stop_history_paging(count, PR_PAGE_SIZE, oldest_in_page, cutoff, rows.len()) {
            break;
        }
        skip += PR_PAGE_SIZE;
    }
    rows.truncate(PR_HISTORY_MAX_COUNT);
    Ok(rows)
}

fn creation_date_unix(item: &Value) -> Option<i64> {
    let text = item["creationDate"].as_str()?;
    parse_rfc3339_unix(text)
}

/// Completed / Abandoned のページングを続けるかどうかの純粋な判定。
/// 「ページが埋まりきらなかった (最終ページ)」「ページ内最古の作成日が
/// cutoff を過ぎた」「累積件数が上限に達した」のいずれかで打ち切る。
fn should_stop_history_paging(
    page_len: usize,
    page_size: usize,
    oldest_in_page: Option<i64>,
    cutoff: i64,
    accumulated_len: usize,
) -> bool {
    let reached_cutoff = oldest_in_page.is_some_and(|oldest| oldest < cutoff);
    page_len < page_size || reached_cutoff || accumulated_len >= PR_HISTORY_MAX_COUNT
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
    let mut body = json!({ "searchText": query, "$skip": 0, "$top": WORK_ITEM_RESULT_LIMIT, "includeFacets": false });
    if !project.interest_areas.is_empty() {
        body["filters"] = json!({ "System.AreaPath": project.interest_areas });
    }
    let value = post_json(client, &url, pat, &body)?;
    Ok(work_item_candidates(project, &value))
}

/// `interest_areas` が設定されていれば `UNDER` 条件で WIQL を絞り込む。
/// 空なら従来どおりプロジェクト全体を対象にする。
fn recent_work_items_wiql(project: &AzureDevOpsProject) -> String {
    let base = "SELECT [System.Id] FROM WorkItems ORDER BY [System.ChangedDate] DESC";
    if project.interest_areas.is_empty() {
        return base.to_string();
    }
    let conditions = project
        .interest_areas
        .iter()
        .map(|area| format!("[System.AreaPath] UNDER '{}'", wiql_escape(area)))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!(
        "SELECT [System.Id] FROM WorkItems WHERE {conditions} ORDER BY [System.ChangedDate] DESC"
    )
}

/// WIQL の文字列リテラル内でシングルクォートをエスケープする。
fn wiql_escape(value: &str) -> String {
    value.replace('\'', "''")
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
        &json!({ "query": recent_work_items_wiql(project) }),
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

/// プロジェクトの Area Path 階層をすべて取得する。設定画面のツリーピッカー用。
/// `typed_pat` が空なら Credential Manager に保存済みの PAT を使う。
pub fn fetch_area_nodes(
    organization: &str,
    project: &str,
    typed_pat: &str,
) -> Result<Vec<AreaNode>, String> {
    let pat = super::credential::credential_for_request(organization, typed_pat)?;
    let client = http_client()?;
    let url = format!(
        "https://dev.azure.com/{}/{}/_apis/wit/classificationnodes/Areas?$depth=12&api-version={API_VERSION}",
        encode_segment(organization),
        encode_segment(project)
    );
    let value = get_json(&client, &url, &pat)?;
    Ok(area_nodes(&value))
}

/// 自分に割り当てられた Work Item から件数の多い順に Area Path を推薦する。
/// 設定画面の Area Path ピッカーで「候補を提案」する材料に使う。
const MY_AREA_SUGGESTION_LIMIT: usize = 200;

pub fn fetch_my_area_suggestions(
    organization: &str,
    project: &str,
    typed_pat: &str,
) -> Result<Vec<(String, usize)>, String> {
    let pat = super::credential::credential_for_request(organization, typed_pat)?;
    let client = http_client()?;
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(organization),
        encode_segment(project)
    );
    let query = post_json(
        &client,
        &format!("{base}/_apis/wit/wiql?$top={MY_AREA_SUGGESTION_LIMIT}&api-version={API_VERSION}"),
        &pat,
        &json!({
            "query": "SELECT [System.Id] FROM WorkItems WHERE [System.AssignedTo] = @Me"
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
        &client,
        &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
        &pat,
        &json!({
            "ids": ids,
            "fields": ["System.AreaPath"],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(area_path_counts(&items))
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

    fn project(interest_areas: Vec<String>) -> AzureDevOpsProject {
        AzureDevOpsProject {
            organization: "org".to_string(),
            project: "project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas,
        }
    }

    #[test]
    fn only_transient_http_statuses_are_retried() {
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }

    #[test]
    fn recent_work_items_wiql_has_no_where_clause_without_interest_areas() {
        let wiql = recent_work_items_wiql(&project(Vec::new()));
        assert!(!wiql.contains("WHERE"));
    }

    #[test]
    fn recent_work_items_wiql_filters_by_interest_areas() {
        let wiql = recent_work_items_wiql(&project(vec![
            "Waypoint\\Launcher".to_string(),
            "Waypoint\\Search".to_string(),
        ]));
        assert!(wiql.contains("[System.AreaPath] UNDER 'Waypoint\\Launcher'"));
        assert!(wiql.contains("[System.AreaPath] UNDER 'Waypoint\\Search'"));
        assert!(wiql.contains(" OR "));
    }

    #[test]
    fn wiql_escape_doubles_single_quotes() {
        assert_eq!(wiql_escape("O'Brien\\Team"), "O''Brien\\Team");
    }

    #[test]
    fn history_paging_continues_while_page_is_full_and_recent() {
        // フルページ (top と同数) かつページ内最古が cutoff より新しければ続行
        assert!(!should_stop_history_paging(500, 500, Some(2_000), 1_000, 10));
    }

    #[test]
    fn history_paging_stops_on_partial_page() {
        // 返ってきた件数が $top 未満 = これが最終ページ
        assert!(should_stop_history_paging(120, 500, Some(2_000), 1_000, 10));
    }

    #[test]
    fn history_paging_stops_once_oldest_item_in_page_predates_cutoff() {
        // ページ内最古の作成日が cutoff (3 ヶ月前など) より古い
        assert!(should_stop_history_paging(500, 500, Some(500), 1_000, 10));
    }

    #[test]
    fn history_paging_stops_at_the_count_cap_even_if_still_recent() {
        assert!(should_stop_history_paging(
            500,
            500,
            Some(2_000),
            1_000,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_continues_when_no_page_item_has_a_parsable_date() {
        // creationDate が読めない行しかない場合は cutoff 判定をスキップし、
        // ページが埋まっていれば続行する (安全側 = 取りこぼさない)
        assert!(!should_stop_history_paging(500, 500, None, 1_000, 10));
    }
}
