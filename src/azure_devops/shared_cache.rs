//! waypoint と DevDeck (別リポジトリの Azure DevOps ダッシュボード) が
//! 同一ユーザーの同一プロジェクトへ独立に同期する際の API 呼び出し重複を
//! 避けるための共有キャッシュ。`%LOCALAPPDATA%\AzDoSharedCache\cache.db`
//! に両アプリが読み書きする。waypoint 自身の `azure_devops.db` は
//! そのまま一次キャッシュとして維持し、ここは「他方のアプリが直近に
//! 同期済みなら自分の API 呼び出しを省略する」ための補助データ。
//! 設計の詳細は `docs/superpowers/specs/2026-08-29-shared-azure-cache-design.md`。

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Connection, params};

use crate::config::AzureDevOpsProject;

use super::cache::CachedRow;
use super::convert::unix_timestamp;

const SHARED_CACHE_DIR: &str = "AzDoSharedCache";
const SHARED_CACHE_FILE: &str = "cache.db";
const WRITTEN_BY: &str = "waypoint";

/// 相手アプリの同期がこの秒数以内なら、自分の Azure DevOps API 呼び出しを
/// 省略して共有キャッシュから読む。定期フル同期の間隔 (12 時間) よりずっと
/// 短くし、鮮度を大きく落とさずに重複同期だけを避ける。
const PEER_FRESH_MAX_AGE_SECS: i64 = 15 * 60;

pub(crate) fn shared_cache_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|path| path.join(SHARED_CACHE_DIR).join(SHARED_CACHE_FILE))
}

fn open_shared_cache() -> Result<Connection, String> {
    let path = shared_cache_path().ok_or_else(|| "AppData path is unavailable.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS shared_candidates (
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
            CREATE TABLE IF NOT EXISTS shared_project_state (
                organization TEXT NOT NULL,
                project TEXT NOT NULL,
                refreshed_at INTEGER,
                written_by TEXT,
                included_pull_requests INTEGER NOT NULL DEFAULT 0,
                included_pipelines INTEGER NOT NULL DEFAULT 0,
                included_work_items INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (organization, project)
            );",
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

/// このプロジェクトの同期結果を共有キャッシュへ書く。失敗しても waypoint
/// 自身の同期は成功しているので、呼び出し側はログに残すだけでよい。
pub(crate) fn publish_project(
    project: &AzureDevOpsProject,
    rows: &[CachedRow],
) -> Result<(), String> {
    let mut connection = open_shared_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM shared_candidates WHERE organization = ?1 AND project = ?2",
            params![project.organization.trim(), project.project.trim()],
        )
        .map_err(|error| error.to_string())?;
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO shared_candidates
                 (organization, project, kind, item_id, status, name, detail, url, is_mine)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(|error| error.to_string())?;
        for row in rows {
            statement
                .execute(params![
                    row.organization,
                    row.project,
                    row.kind,
                    row.item_id,
                    row.status,
                    row.name,
                    row.detail,
                    row.url,
                    row.is_mine as i64,
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction
        .execute(
            "INSERT INTO shared_project_state
             (organization, project, refreshed_at, written_by,
              included_pull_requests, included_pipelines, included_work_items)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(organization, project) DO UPDATE SET
                refreshed_at = excluded.refreshed_at,
                written_by = excluded.written_by,
                included_pull_requests = excluded.included_pull_requests,
                included_pipelines = excluded.included_pipelines,
                included_work_items = excluded.included_work_items",
            params![
                project.organization.trim(),
                project.project.trim(),
                unix_timestamp(),
                WRITTEN_BY,
                project.include_pull_requests as i64,
                project.include_pipelines as i64,
                project.include_work_items as i64,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

/// 他方のアプリが直近に同期済みで、かつ自分がこのプロジェクトで必要とする
/// 範囲 (PR / Pipeline / Work Item) をすべて含んでいれば、その候補行を返す
/// (自分の API 呼び出しを省略してこれを採用してよい、という合図)。
/// 条件を満たさなければ `None` — 呼び出し側は従来どおり自分で同期する。
pub(crate) fn peer_fresh_candidates(project: &AzureDevOpsProject) -> Option<Vec<CachedRow>> {
    let connection = open_shared_cache().ok()?;
    let state = connection
        .query_row(
            "SELECT refreshed_at, written_by, included_pull_requests,
                    included_pipelines, included_work_items
             FROM shared_project_state WHERE organization = ?1 AND project = ?2",
            params![project.organization.trim(), project.project.trim()],
            |row| {
                Ok(PeerState {
                    refreshed_at: row.get::<_, Option<i64>>(0)?,
                    written_by: row.get::<_, Option<String>>(1)?,
                    included_pull_requests: row.get::<_, i64>(2)? != 0,
                    included_pipelines: row.get::<_, i64>(3)? != 0,
                    included_work_items: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .ok()?;
    if !is_peer_fresh_and_sufficient(&state, project, unix_timestamp(), PEER_FRESH_MAX_AGE_SECS) {
        return None;
    }
    let mut statement = connection
        .prepare(
            "SELECT organization, project, kind, item_id, status, name, detail, url, is_mine
             FROM shared_candidates WHERE organization = ?1 AND project = ?2",
        )
        .ok()?;
    let rows = statement
        .query_map(
            params![project.organization.trim(), project.project.trim()],
            |row| {
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
            },
        )
        .ok()?
        .filter_map(Result::ok)
        .collect();
    Some(rows)
}

struct PeerState {
    refreshed_at: Option<i64>,
    written_by: Option<String>,
    included_pull_requests: bool,
    included_pipelines: bool,
    included_work_items: bool,
}

fn is_peer_fresh_and_sufficient(
    state: &PeerState,
    project: &AzureDevOpsProject,
    now: i64,
    max_age_secs: i64,
) -> bool {
    let Some(refreshed_at) = state.refreshed_at else {
        return false;
    };
    let Some(written_by) = state.written_by.as_deref() else {
        return false;
    };
    if written_by == WRITTEN_BY {
        // 自分が最後に書いたものを自分の鮮度判定に使うと、常に「新鮮」と
        // 誤判定して相手の更新を永久に見なくなる。
        return false;
    }
    if now.saturating_sub(refreshed_at) > max_age_secs {
        return false;
    }
    let covers = |needed: bool, has: bool| !needed || has;
    covers(project.include_pull_requests, state.included_pull_requests)
        && covers(project.include_pipelines, state.included_pipelines)
        && covers(project.include_work_items, state.included_work_items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(pr: bool, pipeline: bool, wit: bool) -> AzureDevOpsProject {
        AzureDevOpsProject {
            organization: "org".to_string(),
            project: "proj".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: pr,
            include_pipelines: pipeline,
            include_work_items: wit,
            interest_repositories: Vec::new(),
            interest_areas: Vec::new(),
        }
    }

    fn state(
        refreshed_at: Option<i64>,
        written_by: Option<&str>,
        pr: bool,
        pipeline: bool,
        wit: bool,
    ) -> PeerState {
        PeerState {
            refreshed_at,
            written_by: written_by.map(str::to_string),
            included_pull_requests: pr,
            included_pipelines: pipeline,
            included_work_items: wit,
        }
    }

    /// 相手アプリが直近に PR/Pipeline/WorkItem すべて同期済みなら採用してよい。
    #[test]
    fn adopts_recent_peer_sync_covering_everything_needed() {
        let project = project(true, true, true);
        let state = state(Some(100), Some("devdeck"), true, true, true);
        assert!(is_peer_fresh_and_sufficient(
            &state,
            &project,
            100 + 60,
            900
        ));
    }

    /// 古すぎる同期は採用しない。
    #[test]
    fn rejects_stale_peer_sync() {
        let project = project(true, false, false);
        let state = state(Some(100), Some("devdeck"), true, false, false);
        assert!(!is_peer_fresh_and_sufficient(
            &state,
            &project,
            100 + 1000,
            900
        ));
    }

    /// 自分自身が書いたものは相手の鮮度判定に使わない
    /// (常に新鮮と誤判定して相手の更新を永久に見なくなるのを防ぐ)。
    #[test]
    fn rejects_own_write_as_peer_freshness() {
        let project = project(true, false, false);
        let state = state(Some(100), Some(WRITTEN_BY), true, false, false);
        assert!(!is_peer_fresh_and_sufficient(&state, &project, 100, 900));
    }

    /// 相手が Pipeline を同期していないのに自分が Pipeline も必要なら、
    /// 採用すると `replace_project_cache` の全置換で自分の Pipeline
    /// キャッシュを消してしまうため、採用しない。
    #[test]
    fn rejects_peer_missing_a_kind_this_project_needs() {
        let project = project(true, true, false);
        let state = state(Some(100), Some("devdeck"), true, false, false);
        assert!(!is_peer_fresh_and_sufficient(&state, &project, 100, 900));
    }

    /// 未同期 (行が無い相当) は採用しない。
    #[test]
    fn rejects_never_synced_peer() {
        let project = project(true, false, false);
        let state = state(None, Some("devdeck"), true, false, false);
        assert!(!is_peer_fresh_and_sufficient(&state, &project, 100, 900));
    }
}
