//! SQLite キャッシュの読み書き。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params};

use crate::config::AzureDevOpsProject;

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
    // 複数プロジェクトを thread::scope で並列同期するため、コミットが
    // 重なると既定 (待機なし) では SQLITE_BUSY になりうる。ロック解放を
    // 待つようにして間欠的な同期失敗を防ぐ。
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
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
                work_items_delta_synced_at INTEGER,
                PRIMARY KEY (organization, project)
            );
            CREATE TABLE IF NOT EXISTS cache_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS identity (
                organization TEXT PRIMARY KEY,
                user_id TEXT NOT NULL
            );
            INSERT OR REPLACE INTO cache_meta (key, value) VALUES ('schema_version', '2');",
        )
        .map_err(|error| error.to_string())?;
    // v1 のキャッシュをそのまま移行する。既に列がある場合のエラーは無視する。
    let _ = connection.execute(
        "ALTER TABLE candidates ADD COLUMN is_mine INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE project_state ADD COLUMN work_items_delta_synced_at INTEGER",
        [],
    );
    Ok(connection)
}

pub(crate) fn open_cache_read_only() -> Result<Connection, String> {
    let path = cache_path().ok_or_else(|| "AppData path is unavailable.".to_string())?;
    open_read_only_at(&path)
}

fn open_read_only_at(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
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
    // 'pipeline' / 'wit' はもう書かない (Pipeline は Live 検索のみ、Work
    // Item は DevDeck のキャッシュを読む) が、旧バージョンが残した行を
    // アップグレード後の最初の同期で確実に消すため IN 句には残す。
    transaction
        .execute(
            "DELETE FROM candidates
             WHERE organization = ?1 AND project = ?2 AND kind IN ('pr', 'pipeline', 'wit')",
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

/// フル同期の成功を記録する。
pub(crate) fn record_project_success(project: &AzureDevOpsProject) -> Result<(), String> {
    let connection = open_cache()?;
    let now = unix_timestamp();
    connection
        .execute(
            "INSERT INTO project_state (organization, project, refreshed_at, last_error)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(organization, project) DO UPDATE SET
                refreshed_at = excluded.refreshed_at,
                last_error = NULL",
            params![project.organization.trim(), project.project.trim(), now],
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

/// The authenticated user's identity GUID for `organization`, resolved by
/// `current_user_id` on some earlier sync and cached here so the synchronous
/// read path (`cached_candidates`, no network access allowed) can compute
/// "is this mine" against the shared cache without an API call.
pub(crate) fn read_identity(organization: &str) -> Option<String> {
    let connection = open_cache().ok()?;
    connection
        .query_row(
            "SELECT user_id FROM identity WHERE organization = ?1",
            params![organization.trim()],
            |row| row.get(0),
        )
        .ok()
}

pub(crate) fn write_identity(organization: &str, user_id: &str) -> Result<(), String> {
    let connection = open_cache()?;
    connection
        .execute(
            "INSERT INTO identity (organization, user_id) VALUES (?1, ?2)
             ON CONFLICT(organization) DO UPDATE SET user_id = excluded.user_id",
            params![organization.trim(), user_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join(CACHE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_connection_reads_existing_data_and_rejects_writes() {
        let path = std::env::temp_dir().join(format!(
            "waypoint-cache-readonly-{}-{}.db",
            std::process::id(),
            unix_timestamp()
        ));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("CREATE TABLE sample(value INTEGER NOT NULL)", [])
            .unwrap();
        connection
            .execute("INSERT INTO sample VALUES(42)", [])
            .unwrap();
        drop(connection);

        let connection = open_read_only_at(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert!(
            connection
                .execute("INSERT INTO sample VALUES(7)", [])
                .is_err()
        );
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }
}
