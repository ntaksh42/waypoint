//! SQLite キャッシュの読み書き。

use std::path::PathBuf;

use rusqlite::{Connection, params};

use crate::config::AzureDevOpsProject;

use super::Candidate;
use super::convert::unix_timestamp;

const CACHE_FILE: &str = "azure_devops.db";

#[derive(Debug)]
pub(crate) struct CachedRow {
    pub(crate) organization: String,
    pub(crate) project: String,
    pub(crate) kind: String,
    pub(crate) item_id: String,
    pub(crate) status: String,
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) url: String,
    pub(crate) is_mine: bool,
}

pub(crate) fn open_cache() -> Result<Connection, String> {
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

pub(crate) fn replace_project_cache(
    project: &AzureDevOpsProject,
    rows: &[CachedRow],
) -> Result<(), String> {
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM candidates
             WHERE organization = ?1 AND project = ?2 AND kind IN ('pr', 'pipeline')",
            params![project.organization.trim(), project.project.trim()],
        )
        .map_err(|error| error.to_string())?;
    {
        // 行ごとに `execute` へ SQL 文字列を渡すと rusqlite が毎回 prepare
        // し直す。大量行では無視できないオーバーヘッドになるため、
        // ステートメントをループの外で 1 回だけ用意して使い回す。
        let mut statement = transaction
            .prepare(
                "INSERT INTO candidates
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
    transaction.commit().map_err(|error| error.to_string())
}

/// ライブ検索で見つけた Work Item を次回の即時検索用に保持する。
/// PR / Pipeline 同期時に消されないよう `kind = wit` として別扱いにする。
pub(crate) fn cache_work_item_candidates(candidates: &[Candidate]) -> Result<(), String> {
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for candidate in candidates {
        let Some(item_id) = candidate.url.rsplit('/').next() else {
            continue;
        };
        transaction
            .execute(
                "INSERT OR REPLACE INTO candidates
                 (organization, project, kind, item_id, status, name, detail, url, is_mine)
                 VALUES (?1, ?2, 'wit', ?3, ?4, ?5, ?6, ?7, 0)",
                params![
                    candidate.organization,
                    candidate.project,
                    item_id,
                    candidate.status,
                    candidate.name,
                    candidate.detail,
                    candidate.url,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn record_project_success(project: &AzureDevOpsProject) -> Result<(), String> {
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

pub(crate) fn record_project_error(
    project: &AzureDevOpsProject,
    error: &str,
) -> Result<(), String> {
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

pub(crate) fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join(CACHE_FILE))
}
