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
mod convert;
mod credential;
mod shared_cache;
mod sync;

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use rusqlite::params;

use crate::config::AzureDevOpsSettings;

use api::{API_VERSION, http_client};
use cache::{CachedRow, open_cache, open_cache_read_only, read_identity};
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
    ProjectActivity, PullRequestReply, WorkItemReply, refresh_async, search_pipelines_live_async,
    search_pull_requests_live_async, search_work_items_async, suggest_priorities_async,
    take_pipeline_results, take_pull_request_results, take_work_item_results,
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

/// Active PR 候補。共有キャッシュ (`shared_cache`) を読む。フレッシュで
/// なければ waypoint 自身がバックグラウンドで更新する
/// (`api.rs::refresh_project`) ので、ここは常に「今ある内容」を返すだけ。
pub fn cached_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    cached_candidate_groups(settings).0
}

/// waypoint 自身が保持する PR 履歴 (Completed/Abandoned、過去 90 日ぶん)。
/// DevDeck は Active PR しか同期しないので、この分だけは重複しておらず
/// waypoint 側のキャッシュのまま残す (`api.rs::fetch_pull_requests` 参照)。
/// 読めなければ空で続行する。読み取り専用接続を使い、waypoint 自身の
/// バックグラウンド同期の書き込みと競合しない。
fn pr_history_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    let Ok(connection) = open_cache_read_only() else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT organization, project, kind, item_id, status, name, detail, url, is_mine
         FROM candidates WHERE kind = 'pr'",
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
        .filter(|project| valid_project(project) && project.include_pull_requests)
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                (project.aliases.clone(), project.priority),
            )
        })
        .collect();
    rows.filter_map(Result::ok)
        .filter_map(|row| {
            let (aliases, priority) = configured
                .get(&project_key(&row.organization, &row.project))?
                .clone();
            Some(Candidate {
                kind: Kind::PullRequest,
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

/// Work Item 候補。共有キャッシュ (`shared_cache`) を読む
/// (`az wit live` はライブ検索で API を直接叩くので対象外)。
/// 読めなければ空で続行する。
pub fn cached_work_item_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    cached_candidate_groups(settings).1
}

/// Active PR と Work Item を共有キャッシュ (`shared_cache`) の同じ接続から
/// まとめて読む (接続を開き直すコストを 1 回で済ませる)。is_mine は共有
/// キャッシュに持たない (どちらのアプリの都合でもない生の事実だけを置く
/// 設計) ので、`created_by_id` / レビュアー一覧を自分の
/// `authenticated_user_id` (`cache::read_identity`、ネットワークなしで
/// 同期的に読める) と突き合わせてここで計算する。
pub(crate) fn cached_candidate_groups(
    settings: &AzureDevOpsSettings,
) -> (Vec<Candidate>, Vec<Candidate>) {
    if !settings.enabled {
        return (Vec::new(), Vec::new());
    }
    // 共有キャッシュが開けなくても、waypoint 自身の PR 履歴は別の DB
    // (waypoint 自身の `cache.rs`) なので独立して読める。
    let Ok((mut pull_requests, work_items)) = shared_cache::with_cached_connection(|connection| {
        read_candidate_groups(connection, settings)
    }) else {
        return (pr_history_candidates(settings), Vec::new());
    };
    pull_requests.extend(pr_history_candidates(settings));
    (pull_requests, work_items)
}

/// `cached_candidate_groups` の本体。`healthy` は呼び出し元のキャッシュ
/// 接続を使い回してよいかの判定に使う — スキーマ不一致等でクエリ自体が
/// 失敗した場合だけ `false` にし、次回呼び出しで接続を開き直させる
/// (プロジェクトが未設定/該当行なしはクエリ自体は成功しているので健全)。
fn read_candidate_groups(
    connection: &rusqlite::Connection,
    settings: &AzureDevOpsSettings,
) -> ((Vec<Candidate>, Vec<Candidate>), bool) {
    let mut pull_requests = Vec::new();
    let mut work_items = Vec::new();
    let mut healthy = true;
    for project in &settings.projects {
        if !valid_project(project) {
            continue;
        }
        let organization = project.organization.trim();
        let project_name = project.project.trim();

        if project.include_pull_requests {
            match shared_cache::read_pull_requests(connection, organization, project_name) {
                Ok(rows) => {
                    let reviewers =
                        shared_cache::read_reviewers(connection, organization, project_name)
                            .unwrap_or_default();
                    let my_id = read_identity(organization);
                    for row in rows {
                        let Some(url) = row.web_url else { continue };
                        let is_mine = my_id.as_deref().is_some_and(|my_id| {
                            row.created_by_id.as_deref() == Some(my_id)
                                || reviewers.iter().any(|reviewer| {
                                    reviewer.repository_id == row.repository_id
                                        && reviewer.pull_request_id == row.pull_request_id
                                        && reviewer.reviewer_id == my_id
                                })
                        });
                        pull_requests.push(Candidate {
                            kind: Kind::PullRequest,
                            name: format!("PR {}: {}", row.pull_request_id, row.title),
                            detail: match &row.created_by {
                                Some(author) if !author.is_empty() => format!(
                                    "Azure DevOps — {organization}/{project_name} — {} — by {author}",
                                    row.status
                                ),
                                _ => format!(
                                    "Azure DevOps — {organization}/{project_name} — {}",
                                    row.status
                                ),
                            },
                            status: row.status,
                            url,
                            organization: organization.to_string(),
                            project: project_name.to_string(),
                            aliases: project.aliases.clone(),
                            priority: project.priority,
                            is_mine,
                        });
                    }
                }
                Err(_) => healthy = false,
            }
        }

        if project.include_work_items {
            match shared_cache::read_work_items(connection, organization, project_name) {
                Ok(rows) => {
                    for row in rows {
                        let Some(url) = row.web_url else { continue };
                        let kind = row.work_item_type.as_deref().unwrap_or("Work Item");
                        let state = row.state.as_deref().unwrap_or("");
                        work_items.push(Candidate {
                            kind: Kind::WorkItem,
                            status: state.to_string(),
                            name: format!("{}: {}", row.id, row.title),
                            detail: format!(
                                "Azure DevOps — {organization}/{project_name} — {kind} {state}"
                            ),
                            url,
                            organization: organization.to_string(),
                            project: project_name.to_string(),
                            aliases: project.aliases.clone(),
                            priority: project.priority,
                            is_mine: false,
                        });
                    }
                }
                Err(_) => healthy = false,
            }
        }
    }
    ((pull_requests, work_items), healthy)
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
