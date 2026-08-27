//! Azure DevOps の Quick Launch 連携。
//!
//! PR とパイプラインは SQLite キャッシュだけを検索し、Work Item だけは
//! `az wit ` 中にバックグラウンドで API 検索する。PAT は設定ファイルへ
//! 書かず、Windows Credential Manager (`keyring`) から必要時だけ読む。

mod api;
mod cache;
mod convert;
mod credential;
mod sync;

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use rusqlite::params;

use crate::config::AzureDevOpsSettings;

use api::{API_VERSION, http_client};
use cache::{CachedRow, open_cache};
use convert::{
    encode_segment, project_key, project_names, project_url, sort_and_deduplicate_project_names,
    unix_timestamp, valid_project,
};
use credential::credential_for_request;
use sync::REFRESHING;

pub use api::{fetch_area_nodes, fetch_my_area_suggestions, list_repository_names};
pub use convert::AreaNode;
pub use credential::{delete_pat, save_pat};
pub use sync::{
    ProjectActivity, PullRequestReply, WorkItemReply, refresh_async,
    refresh_work_items_delta_async, search_pull_requests_live_async, search_work_items_async,
    suggest_priorities_async, take_pull_request_results, take_work_item_results,
};

const PROJECT_PAGE_SIZE: usize = 1_000;

/// Quick Launch の `az optimize`（`suggest` / `rank` でも入れる）から
/// `waypoint-settings.exe` を起動する際に渡すコマンドライン引数。設定
/// エディターはこれを見て、起動直後に優先度提案モーダルを自動で開く
/// (`az optimize` → 別プロセス起動 → 直後に集計開始、という導線を
/// 常駐部と設定エディターの両方で共有するための定数)。
pub const AZURE_SUGGEST_ARG: &str = "--azure-suggest";

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

    /// ライブ検索 (`search_pull_requests_live_async`) で API に渡す
    /// `searchCriteria.status` の値。`All` は Active/Completed/Abandoned の
    /// 全ステータスを順に叩く (呼び出し側が結果をまとめる)。Active も含める
    /// のは、監視対象外プロジェクトや未同期などでキャッシュに無いことが
    /// あり、ライブ検索まで来た時点では「確実に見つける」を優先するため。
    pub fn live_search_statuses(self) -> &'static [&'static str] {
        match self {
            Self::All => &["active", "completed", "abandoned"],
            Self::Active => &["active"],
            Self::Completed => &["completed"],
            Self::Abandoned => &["abandoned"],
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

/// Quick Launch と設定画面に出す、キャッシュの鮮度と最後の同期結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStatus {
    pub refreshed_at: Option<i64>,
    pub failed_projects: usize,
    pub last_error: Option<String>,
    pub refresh_in_progress: bool,
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
         FROM candidates WHERE kind IN ('pr', 'pipeline')",
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

/// 永続キャッシュから Work Item 候補を読む。Quick Launch 表示時はメモリ上の
/// この結果だけを検索し、キャッシュで見つからない場合にだけ API を呼ぶ。
pub fn cached_work_item_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    if !settings.enabled {
        return Vec::new();
    }
    let Ok(connection) = open_cache() else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT organization, project, status, name, detail, url
         FROM candidates WHERE kind = 'wit' ORDER BY rowid DESC",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    }) else {
        return Vec::new();
    };
    let configured: HashMap<_, _> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project) && project.include_work_items)
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                (project.aliases.clone(), project.priority),
            )
        })
        .collect();
    rows.filter_map(Result::ok)
        .filter_map(|(organization, project, status, name, detail, url)| {
            let (aliases, priority) = configured
                .get(&project_key(&organization, &project))?
                .clone();
            Some(Candidate {
                kind: Kind::WorkItem,
                status,
                name,
                detail,
                url,
                organization,
                project,
                aliases,
                priority,
                is_mine: false,
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

/// Work Item 差分同期の直近実行時刻 (全監視プロジェクトのうち最も古いもの)。
/// Quick Launch ウィンドウを開いた瞬間のキックにクールダウンをかける判定に使う。
/// DB が無い・どのプロジェクトも一度も同期していなければ `None`
/// (呼び出し側はクールダウンなしでキックしてよい —
/// フル同期がまだ起点を作っていなければ差分同期自体が何もしないので無害)。
pub fn work_items_delta_synced_at(settings: &AzureDevOpsSettings) -> Option<i64> {
    let connection = open_cache().ok()?;
    settings
        .projects
        .iter()
        .filter(|project| valid_project(project))
        .filter_map(|project| {
            connection
                .query_row(
                    "SELECT work_items_delta_synced_at FROM project_state
                     WHERE organization = ?1 AND project = ?2",
                    params![project.organization.trim(), project.project.trim()],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
        })
        .min()
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
    api::get_json(&http_client()?, &url, &pat).map(|_| ())
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
        let value = api::get_json(&client, &url, &pat)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_status_filters_are_exact() {
        assert!(PullRequestStatus::Active.matches("active"));
        assert!(PullRequestStatus::Completed.matches("completed"));
        assert!(!PullRequestStatus::Completed.matches("abandoned"));
        assert!(PullRequestStatus::All.matches("anything"));
    }

    /// `az pr live 45` のように未同期・監視対象外プロジェクトの Active な
    /// PR を検索したとき、live 検索が active を叩かないと絶対にヒットし
    /// ない (キャッシュに全件あるはずという前提はライブ検索の存在意義と
    /// 矛盾する)。回帰防止。
    #[test]
    fn live_search_statuses_include_active_so_live_search_can_actually_find_it() {
        assert_eq!(
            PullRequestStatus::All.live_search_statuses(),
            &["active", "completed", "abandoned"]
        );
        assert_eq!(
            PullRequestStatus::Active.live_search_statuses(),
            &["active"]
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
}
