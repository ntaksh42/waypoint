//! Azure DevOps の Quick Launch 連携。
//!
//! Active PR と Work Item は waypoint と DevDeck (別リポジトリの Azure
//! DevOps ダッシュボード) が共有する中立な SQLite キャッシュ
//! (`shared_cache`、`%APPDATA%\AzDoSharedCache\cache.db`) に対して読み書き
//! する。どちらのアプリの内部スキーマにも依存しない — 同じ組織を独立に
//! ポーリングして API 呼び出しが重複するのを避けるための共有先で、直近
//! (自分でも相手でも) 更新済みなら自分の取得をスキップする
//! (`api.rs::refresh_project` 参照)。PR の Completed/Abandoned 履歴は
//! DevDeck の対象外なので waypoint 自身の SQLite キャッシュ (`cache`) の
//! まま。Pipeline は永続キャッシュを持たず、`az pipeline ` の明示的な
//! 選択でだけ Live 検索する。PAT は設定ファイルへ書かず、Windows
//! Credential Manager (`keyring`) から必要時だけ読む。

mod api;
mod auth_cache;
mod cache;
mod candidate_cache;
mod convert;
mod credential;
mod shared_cache;
mod sync;
mod title_search;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use rusqlite::params;

use crate::config::AzureDevOpsSettings;

use api::{API_VERSION, http_client};
use cache::open_cache;
use convert::{
    encode_segment, project_key, project_names, project_url, sort_and_deduplicate_project_names,
    unix_timestamp, valid_project,
};
use credential::credential_for_request;
use sync::REFRESHING;
pub(crate) use title_search::match_quality as title_match_quality;

pub use api::{fetch_area_nodes, fetch_my_area_suggestions, list_repository_names};
pub(crate) use candidate_cache::cached_candidate_groups;
// 後続の Azure 更新処理が親モジュール経由で使う crate 内 API。
#[allow(unused_imports)]
pub(crate) use candidate_cache::{CachedCandidateGroups, try_cached_candidate_groups};
pub use candidate_cache::{cached_candidates, cached_work_item_candidates};
pub use convert::AreaNode;
pub use credential::{delete_pat, save_pat};
pub(crate) use sync::take_refresh_reply;
pub use sync::{
    ProjectActivity, PullRequestReply, WorkItemReply, apply_optimization, refresh_async,
    search_pipelines_live_async, search_pull_requests_live_async, search_work_items_async,
    suggest_priorities_async, take_pipeline_results, take_pull_request_results,
    take_work_item_results,
};

const PROJECT_PAGE_SIZE: usize = 1_000;

/// 設定エディターを外部から起動したとき、従来の優先度提案モーダルを
/// 自動表示するためのコマンドライン引数。`az optimize` 自体は使わない。
pub const AZURE_SUGGEST_ARG: &str = "--azure-suggest";

static OPTIMIZING: AtomicBool = AtomicBool::new(false);

/// `az optimize` 用。ネットワーク集計と保存を UI スレッド外で完結させ、成功時
/// だけ設定再読み込みメッセージを返す。同じ最適化を同時に走らせない。
pub fn optimize_async(settings: AzureDevOpsSettings, notify: HWND, message: u32) -> bool {
    if OPTIMIZING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let notify = notify.0 as isize;
    thread::spawn(move || {
        let result = suggest_priorities_async(settings)
            .recv()
            .map_err(|_| "Recent activity worker stopped unexpectedly.".to_string())
            .and_then(|result| result)
            .and_then(|activity| match crate::config::load() {
                crate::config::LoadOutcome::Loaded(mut config)
                | crate::config::LoadOutcome::Created(mut config) => {
                    let optimized = apply_optimization(
                        &mut config.settings.quick_launch.azure_devops,
                        &activity,
                    );
                    if optimized == 0 {
                        return Err(
                            "No recent work item activity found in the last 90 days.".to_string()
                        );
                    }
                    crate::config::save(&config)
                        .map_err(|error| format!("Could not save optimization: {error}"))?;
                    Ok(optimized)
                }
                crate::config::LoadOutcome::Failed(error) => Err(error),
            });
        match result {
            Ok(optimized) => {
                crate::panic_log::record(&format!(
                    "azure devops: optimized {optimized} project(s) from recent activity"
                ));
                unsafe {
                    let _ =
                        PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
                }
            }
            Err(error) => {
                crate::panic_log::record(&format!("azure devops: optimize failed: {error}"))
            }
        }
        OPTIMIZING.store(false, Ordering::Release);
    });
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    PullRequest,
    Pipeline,
    WorkItem,
}

/// `az pipeline ` の絞り込み。Pipeline は永続キャッシュを持たないので
/// (`search_pipelines_live_async` 参照)、ここでの分類はライブ検索結果への
/// ローカルフィルタとして使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFilter {
    All,
    Definitions,
    Failed,
}

impl PipelineFilter {
    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::All => true,
            Self::Definitions => status.eq_ignore_ascii_case("definition"),
            Self::Failed => status.eq_ignore_ascii_case("failed"),
        }
    }
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
