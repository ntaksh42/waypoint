//! Azure DevOps の Quick Launch 連携。
//!
//! PR とパイプラインは SQLite キャッシュだけを検索し、Work Item だけは
//! `az wit ` 中にバックグラウンドで API 検索する。PAT は設定ファイルへ
//! 書かず、Windows Credential Manager (`keyring`) から必要時だけ読む。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD};
use keyring::Entry as CredentialEntry;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::{AzureDevOpsProject, AzureDevOpsSettings};

const CREDENTIAL_SERVICE: &str = "Waypoint";
const CACHE_FILE: &str = "azure_devops.db";
const API_VERSION: &str = "7.1";
const PR_PAGE_SIZE: usize = 500;
const PROJECT_PAGE_SIZE: usize = 1_000;
const PIPELINE_BUILD_LIMIT: usize = 100;
const WORK_ITEM_RESULT_LIMIT: usize = 50;
const REQUEST_RETRIES: usize = 2;
const RETRY_DELAY: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    PullRequest,
    Pipeline,
    WorkItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestStatus {
    All,
    Active,
    Completed,
    Abandoned,
}

impl PullRequestStatus {
    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::All => true,
            Self::Active => status.eq_ignore_ascii_case("active"),
            Self::Completed => status.eq_ignore_ascii_case("completed"),
            Self::Abandoned => status.eq_ignore_ascii_case("abandoned"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    pub status: String,
    pub name: String,
    pub detail: String,
    pub url: String,
    pub organization: String,
    pub project: String,
    pub aliases: Vec<String>,
    pub priority: u32,
    pub is_mine: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WorkItemReply {
    pub candidates: Vec<Candidate>,
    pub message: Option<String>,
}

/// Quick Launch と設定画面に出す、キャッシュの鮮度と最後の同期結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStatus {
    pub refreshed_at: Option<i64>,
    pub failed_projects: usize,
    pub last_error: Option<String>,
    pub refresh_in_progress: bool,
}

/// 同期を一つに直列化する。設定保存と手動更新が重なっても API と DB を競合させない。
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Credential Manager 内の組織固有キー。PAT の値は config.json に現れない。
fn credential_key(organization: &str) -> String {
    format!("azure-devops:{}", organization.trim().to_ascii_lowercase())
}

pub fn save_pat(organization: &str, pat: &str) -> Result<(), String> {
    let organization = organization.trim();
    if organization.is_empty() || pat.trim().is_empty() {
        return Err("Organization and PAT are required.".to_string());
    }
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .set_password(pat.trim())
        .map_err(|error| format!("Failed to save PAT: {error}"))
}

pub fn delete_pat(organization: &str) -> Result<(), String> {
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization.trim()))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .delete_credential()
        .or_else(|error| match error {
            keyring::Error::NoEntry => Ok(()),
            error => Err(error),
        })
        .map_err(|error| format!("Failed to delete PAT: {error}"))
}

fn load_pat(organization: &str) -> Result<String, String> {
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .get_password()
        .map_err(|_| format!("No PAT is saved for Azure DevOps organization \"{organization}\"."))
}

/// 設定済みプロジェクトはキャッシュなしでも `az project` で開けるようにする。
pub fn project_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    settings
        .projects
        .iter()
        .filter(|project| valid_project(project))
        .map(|project| Candidate {
            kind: Kind::Project,
            status: String::new(),
            name: project.project.trim().to_string(),
            detail: format!("Azure DevOps — {}", project.organization.trim()),
            url: project_url(project),
            organization: project.organization.trim().to_string(),
            project: project.project.trim().to_string(),
            aliases: project.aliases.clone(),
            priority: project.priority,
            is_mine: false,
        })
        .collect()
}

/// 永続キャッシュから PR / Pipeline 候補を読む。読めなければ空で続行する。
pub fn cached_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    if !settings.enabled {
        return Vec::new();
    }
    let Ok(connection) = open_cache() else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT organization, project, kind, item_id, status, name, detail, url, is_mine
         FROM candidates",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok(CachedRow {
            organization: row.get(0)?,
            project: row.get(1)?,
            kind: row.get(2)?,
            item_id: row.get(3)?,
            status: row.get(4)?,
            name: row.get(5)?,
            detail: row.get(6)?,
            url: row.get(7)?,
            is_mine: row.get::<_, i64>(8)? != 0,
        })
    }) else {
        return Vec::new();
    };

    let configured: HashMap<_, _> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project))
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                (
                    project.aliases.clone(),
                    project.priority,
                    project.include_pull_requests,
                    project.include_pipelines,
                ),
            )
        })
        .collect();
    rows.filter_map(Result::ok)
        .filter_map(|row| {
            let (aliases, priority, include_pull_requests, include_pipelines) = configured
                .get(&project_key(&row.organization, &row.project))?
                .clone();
            if (row.kind == "pr" && !include_pull_requests)
                || (row.kind != "pr" && !include_pipelines)
            {
                return None;
            }
            Some(Candidate {
                kind: if row.kind == "pr" {
                    Kind::PullRequest
                } else {
                    Kind::Pipeline
                },
                status: row.status,
                name: row.name,
                detail: row.detail,
                url: row.url,
                organization: row.organization,
                project: row.project,
                aliases,
                priority,
                is_mine: row.is_mine,
            })
        })
        .collect()
}

/// 監視対象の最後の同期状態。DB が無い・壊れている場合も空状態として扱う。
pub fn cache_status(settings: &AzureDevOpsSettings) -> CacheStatus {
    let Ok(connection) = open_cache() else {
        return CacheStatus {
            refresh_in_progress: REFRESHING.load(Ordering::Relaxed),
            ..Default::default()
        };
    };
    let configured: Vec<_> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project))
        .collect();
    let mut refreshed_at: Option<i64> = None;
    let mut failed_projects = 0;
    let mut last_error = None;
    for project in configured {
        let row = connection.query_row(
            "SELECT refreshed_at, last_error FROM project_state WHERE organization = ?1 AND project = ?2",
            params![project.organization.trim(), project.project.trim()],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<String>>(1)?)),
        );
        if let Ok((updated, error)) = row {
            refreshed_at = match (refreshed_at, updated) {
                (Some(previous), Some(updated)) => Some(previous.min(updated)),
                (None, updated) => updated,
                (previous, None) => previous,
            };
            if let Some(error) = error {
                failed_projects += 1;
                last_error = Some(error);
            }
        }
    }
    CacheStatus {
        refreshed_at,
        failed_projects,
        last_error,
        refresh_in_progress: REFRESHING.load(Ordering::Relaxed),
    }
}

/// UI 用の短い鮮度表示。時刻がまだ無ければ、初回同期前であることを示す。
pub fn cache_status_label(status: &CacheStatus) -> String {
    let freshness = status.refreshed_at.map_or_else(
        || "Not refreshed yet".to_string(),
        |updated| {
            let age = unix_timestamp().saturating_sub(updated);
            if age < 60 {
                "Updated just now".to_string()
            } else if age < 3600 {
                format!("Updated {}m ago", age / 60)
            } else {
                format!("Updated {}h ago", age / 3600)
            }
        },
    );
    if status.refresh_in_progress {
        format!("{freshness} — refreshing…")
    } else if status.failed_projects > 0 {
        format!("{freshness} — {} project(s) failed", status.failed_projects)
    } else {
        freshness
    }
}

/// 起動時・設定再読み込み時に呼ぶ。ネットワークと SQLite 更新は専用スレッドで行う。
pub fn refresh_async(settings: AzureDevOpsSettings, notify: HWND, message: u32) -> bool {
    if !settings.enabled || settings.projects.is_empty() {
        return false;
    }
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let notify = notify.0 as isize;
    thread::spawn(move || {
        match http_client() {
            Ok(client) => {
                for project in settings
                    .projects
                    .iter()
                    .filter(|project| valid_project(project))
                {
                    let Ok(pat) = load_pat(&project.organization) else {
                        let _ =
                            record_project_error(project, "No PAT is saved for this organization.");
                        continue;
                    };
                    if let Err(error) = refresh_project(&client, project, &pat) {
                        crate::panic_log::record(&format!(
                            "azure devops: refresh {}/{} failed: {error}",
                            project.organization, project.project
                        ));
                        let _ = record_project_error(project, &error);
                    }
                }
            }
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: could not initialize refresh client: {error}"
            )),
        }
        REFRESHING.store(false, Ordering::Release);
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
        }
    });
    true
}

/// `az wit ` の検索をバックグラウンドで実行する。結果は ID ごとに保持し、
/// 呼び出し側が最新 ID と一致したものだけを表示する。
pub fn search_work_items_async(
    settings: AzureDevOpsSettings,
    query: String,
    request_id: u32,
    notify: HWND,
    message: u32,
) {
    let notify = notify.0 as isize;
    thread::spawn(move || {
        let mut results = Vec::new();
        let mut failures = Vec::new();
        if !query.trim().is_empty() {
            match http_client() {
                Ok(client) => {
                    for project in settings
                        .projects
                        .iter()
                        .filter(|project| valid_project(project) && project.include_work_items)
                    {
                        let Ok(pat) = load_pat(&project.organization) else {
                            failures.push(format!("{}: no PAT", project.organization));
                            continue;
                        };
                        match fetch_work_items(&client, project, &pat, &query) {
                            Ok(mut found) => results.append(&mut found),
                            Err(error) => {
                                crate::panic_log::record(&format!(
                                    "azure devops: work item search {}/{} failed: {error}",
                                    project.organization, project.project
                                ));
                                failures
                                    .push(format!("{}/{}", project.organization, project.project));
                            }
                        }
                    }
                }
                Err(error) => crate::panic_log::record(&format!(
                    "azure devops: could not initialize work item client: {error}"
                )),
            }
        }
        results.sort_by_key(|candidate| (candidate.priority, candidate.name.to_lowercase()));
        let empty_message = if results.is_empty() {
            if failures.is_empty() {
                Some("No matching work items.".to_string())
            } else {
                Some(format!(
                    "Azure DevOps search unavailable ({})",
                    failures.join(", ")
                ))
            }
        } else {
            None
        };
        let mut pending = pending_work_items()
            .lock()
            .expect("work item result lock poisoned");
        pending.insert(
            request_id,
            WorkItemReply {
                candidates: results,
                message: empty_message,
            },
        );
        pending.retain(|id, _| *id >= request_id.saturating_sub(3));
        drop(pending);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(notify as *mut _)),
                message,
                WPARAM(request_id as usize),
                LPARAM(0),
            );
        }
    });
}

pub fn take_work_item_results(request_id: u32) -> Option<WorkItemReply> {
    pending_work_items().lock().ok()?.remove(&request_id)
}

pub fn test_connection(organization: &str, pat: &str) -> Result<(), String> {
    let organization = organization.trim();
    if organization.is_empty() {
        return Err("Organization is required.".to_string());
    }
    let pat = credential_for_request(organization, pat)?;
    let url = format!(
        "https://dev.azure.com/{}/_apis/projects?$top=1&api-version={API_VERSION}",
        encode_segment(organization)
    );
    get_json(&http_client()?, &url, &pat).map(|_| ())
}

/// 組織内の利用可能なプロジェクト名を返す。PAT 入力欄が空なら、既に
/// Credential Manager へ保存した PAT を使うので Save PAT の直後に取得できる。
pub fn list_projects(organization: &str, pat: &str) -> Result<Vec<String>, String> {
    let organization = organization.trim();
    if organization.is_empty() {
        return Err("Organization is required.".to_string());
    }
    let pat = credential_for_request(organization, pat)?;
    let client = http_client()?;
    let mut names = Vec::new();
    let mut skip = 0;
    loop {
        let url = format!(
            "https://dev.azure.com/{}/_apis/projects?stateFilter=wellFormed&$top={PROJECT_PAGE_SIZE}&$skip={skip}&api-version={API_VERSION}",
            encode_segment(organization)
        );
        let value = get_json(&client, &url, &pat)?;
        let page = project_names(&value);
        let count = page.len();
        if count == 0 {
            break;
        }
        names.extend(page);
        skip += count;
    }
    sort_and_deduplicate_project_names(&mut names);
    Ok(names)
}

fn credential_for_request(organization: &str, typed_pat: &str) -> Result<String, String> {
    if typed_pat.trim().is_empty() {
        load_pat(organization)
    } else {
        Ok(typed_pat.trim().to_string())
    }
}

fn project_names(value: &Value) -> Vec<String> {
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

fn sort_and_deduplicate_project_names(names: &mut Vec<String>) {
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
}

fn refresh_project(
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

fn pull_request_row(
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

fn current_user_id(
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

fn pipeline_definition_row(project: &AzureDevOpsProject, item: &Value) -> Option<CachedRow> {
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

fn pipeline_build_row(project: &AzureDevOpsProject, item: &Value) -> Option<CachedRow> {
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

fn pipeline_row(
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

fn fetch_work_items(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    query: &str,
) -> Result<Vec<Candidate>, String> {
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

fn work_item_candidates(project: &AzureDevOpsProject, value: &Value) -> Vec<Candidate> {
    value["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let fields = &item["fields"];
            let id = json_i64(&fields["System.Id"]).or_else(|| json_i64(&item["id"]))?;
            let title = fields["System.Title"]
                .as_str()
                .or_else(|| item["name"].as_str())
                .unwrap_or("Untitled work item");
            let state = fields["System.State"].as_str().unwrap_or("");
            let kind = fields["System.WorkItemType"]
                .as_str()
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
        })
        .collect()
}

/// Search API は `System.Id` を数値または文字列で返す。どちらでも候補を
/// 捨てないよう、数値化をここへ閉じ込める。
fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn get_json(client: &reqwest::blocking::Client, url: &str, pat: &str) -> Result<Value, String> {
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

fn post_json(
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

fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("Could not initialize Azure DevOps client: {error}"))
}

fn authorization(pat: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!(":{pat}")))
}

fn open_cache() -> Result<Connection, String> {
    let path = cache_path().ok_or_else(|| "AppData path is unavailable.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS candidates (
                organization TEXT NOT NULL,
                project TEXT NOT NULL,
                kind TEXT NOT NULL,
                item_id TEXT NOT NULL,
                status TEXT NOT NULL,
                name TEXT NOT NULL,
                detail TEXT NOT NULL,
                url TEXT NOT NULL,
                is_mine INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (organization, project, kind, item_id)
            );
            CREATE TABLE IF NOT EXISTS project_state (
                organization TEXT NOT NULL,
                project TEXT NOT NULL,
                refreshed_at INTEGER,
                last_error TEXT,
                PRIMARY KEY (organization, project)
            );
            CREATE TABLE IF NOT EXISTS cache_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR REPLACE INTO cache_meta (key, value) VALUES ('schema_version', '2');",
        )
        .map_err(|error| error.to_string())?;
    // v1 のキャッシュをそのまま移行する。既に列がある場合のエラーは無視する。
    let _ = connection.execute(
        "ALTER TABLE candidates ADD COLUMN is_mine INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(connection)
}

fn replace_project_cache(project: &AzureDevOpsProject, rows: &[CachedRow]) -> Result<(), String> {
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM candidates WHERE organization = ?1 AND project = ?2",
            params![project.organization.trim(), project.project.trim()],
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        transaction
            .execute(
                "INSERT INTO candidates
                 (organization, project, kind, item_id, status, name, detail, url, is_mine)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.organization,
                    row.project,
                    row.kind,
                    row.item_id,
                    row.status,
                    row.name,
                    row.detail,
                    row.url,
                    row.is_mine as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn record_project_success(project: &AzureDevOpsProject) -> Result<(), String> {
    let connection = open_cache()?;
    connection
        .execute(
            "INSERT INTO project_state (organization, project, refreshed_at, last_error)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(organization, project) DO UPDATE SET refreshed_at = excluded.refreshed_at, last_error = NULL",
            params![project.organization.trim(), project.project.trim(), unix_timestamp()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_project_error(project: &AzureDevOpsProject, error: &str) -> Result<(), String> {
    let connection = open_cache()?;
    connection
        .execute(
            "INSERT INTO project_state (organization, project, refreshed_at, last_error)
             VALUES (?1, ?2, NULL, ?3)
             ON CONFLICT(organization, project) DO UPDATE SET last_error = excluded.last_error",
            params![project.organization.trim(), project.project.trim(), error],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// 設定から外れたプロジェクトの候補と状態をまとめて削除する。
pub fn prune_cache(settings: &AzureDevOpsSettings) -> Result<(), String> {
    let connection = open_cache()?;
    let configured: Vec<_> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project))
        .map(|project| project_key(&project.organization, &project.project))
        .collect();
    let mut statement = connection
        .prepare("SELECT DISTINCT organization, project FROM candidates UNION SELECT organization, project FROM project_state")
        .map_err(|error| error.to_string())?;
    let rows: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect();
    for (organization, project) in rows {
        if !configured.contains(&project_key(&organization, &project)) {
            connection
                .execute(
                    "DELETE FROM candidates WHERE organization = ?1 AND project = ?2",
                    params![organization, project],
                )
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "DELETE FROM project_state WHERE organization = ?1 AND project = ?2",
                    params![organization, project],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join(CACHE_FILE))
}

fn project_url(project: &AzureDevOpsProject) -> String {
    format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(project.organization.trim()),
        encode_segment(project.project.trim())
    )
}

fn project_key(organization: &str, project: &str) -> String {
    format!(
        "{}/{}",
        organization.trim().to_lowercase(),
        project.trim().to_lowercase()
    )
}

fn valid_project(project: &AzureDevOpsProject) -> bool {
    !project.organization.trim().is_empty() && !project.project.trim().is_empty()
}

fn encode_segment(value: &str) -> String {
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

#[derive(Debug)]
struct CachedRow {
    organization: String,
    project: String,
    kind: String,
    item_id: String,
    status: String,
    name: String,
    detail: String,
    url: String,
    is_mine: bool,
}

fn pending_work_items() -> &'static Mutex<HashMap<u32, WorkItemReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, WorkItemReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "System.Id": 42,
                "System.Title": "Fix launcher",
                "System.State": "Active",
                "System.WorkItemType": "Bug"
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
                "System.Id": "73", "System.Title": "Fix WIT search"
            }}]}),
        );
        assert_eq!(results[0].name, "73: Fix WIT search");
    }

    #[test]
    fn pull_request_status_filters_are_exact() {
        assert!(PullRequestStatus::Active.matches("active"));
        assert!(PullRequestStatus::Completed.matches("completed"));
        assert!(!PullRequestStatus::Completed.matches("abandoned"));
        assert!(PullRequestStatus::All.matches("anything"));
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
    fn cache_status_label_reports_staleness_and_failures() {
        let label = cache_status_label(&CacheStatus {
            refreshed_at: Some(unix_timestamp().saturating_sub(120)),
            failed_projects: 1,
            last_error: Some("forbidden".to_string()),
            refresh_in_progress: false,
        });
        assert!(label.contains("2m ago"));
        assert!(label.contains("1 project(s) failed"));
    }

    #[test]
    fn only_transient_http_statuses_are_retried() {
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }
}
