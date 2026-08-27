//! SQLite キャッシュの読み書き。

use std::path::PathBuf;
use std::time::Duration;

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

pub(crate) fn replace_project_cache(
    project: &AzureDevOpsProject,
    rows: &[CachedRow],
) -> Result<(), String> {
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    // 'wit' もここで入れ替える。ライブ検索 (cache_work_item_candidates) が
    // 書いた一時的な行は、次の定期同期で「最近更新された Work Item」の
    // 一覧に置き換わる想定でよい (まだ更新中なら定期同期にも出てくる)。
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

/// ライブ全文検索 (`az wit <query>` でキャッシュに無く、ユーザーが明示的に
/// 追加取得を選んだ場合) で見つけた Work Item を、次回の即時検索用に一時
/// 保持する。次の定期同期 (`replace_project_cache`) でこの行は消え、
/// 「最近更新された Work Item」一覧に置き換わる。
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

/// フル同期の成功を記録する。`work_items_delta_synced_at` も同じ時刻へ
/// 進める — フル同期は Work Item を含めて取り直しているので、以後の
/// 差分同期はここを新しい起点にしてよい (フルの直後に古い起点で差分を
/// 取ると、フルで既に反映済みの変更を二重に取得しにいくだけになる)。
pub(crate) fn record_project_success(project: &AzureDevOpsProject) -> Result<(), String> {
    let connection = open_cache()?;
    let now = unix_timestamp();
    connection
        .execute(
            "INSERT INTO project_state (organization, project, refreshed_at, last_error, work_items_delta_synced_at)
             VALUES (?1, ?2, ?3, NULL, ?3)
             ON CONFLICT(organization, project) DO UPDATE SET
                refreshed_at = excluded.refreshed_at,
                last_error = NULL,
                work_items_delta_synced_at = excluded.work_items_delta_synced_at",
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

/// Work Item の差分同期がいつ最後に成功したか。無ければ `None`
/// (まだ一度もフル/差分同期していないので、呼び出し側はフル同期に倒す)。
pub(crate) fn work_items_delta_synced_at(project: &AzureDevOpsProject) -> Option<i64> {
    let connection = open_cache().ok()?;
    connection
        .query_row(
            "SELECT work_items_delta_synced_at FROM project_state
             WHERE organization = ?1 AND project = ?2",
            params![project.organization.trim(), project.project.trim()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
}

pub(crate) fn record_work_items_delta_success(project: &AzureDevOpsProject) -> Result<(), String> {
    let connection = open_cache()?;
    connection
        .execute(
            "INSERT INTO project_state (organization, project, work_items_delta_synced_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(organization, project) DO UPDATE SET work_items_delta_synced_at = excluded.work_items_delta_synced_at",
            params![
                project.organization.trim(),
                project.project.trim(),
                unix_timestamp()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Work Item の差分同期結果を UPSERT する。全置換 (`replace_project_cache`)
/// と違い、今回の応答に含まれない既存行は削除しない — 差分クエリは
/// 「変更があったもの」しか返さないため、含まれない行は「変更が無かった」
/// のか「削除・対象外になった」のか区別できない。削除検知は定期的な
/// フル同期 (`replace_project_cache`) に任せる。
pub(crate) fn upsert_work_item_rows(rows: &[CachedRow]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO candidates
                 (organization, project, kind, item_id, status, name, detail, url, is_mine)
                 VALUES (?1, ?2, 'wit', ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(organization, project, kind, item_id) DO UPDATE SET
                    status = excluded.status,
                    name = excluded.name,
                    detail = excluded.detail,
                    url = excluded.url,
                    is_mine = excluded.is_mine",
            )
            .map_err(|error| error.to_string())?;
        for row in rows {
            statement
                .execute(params![
                    row.organization,
                    row.project,
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

pub(crate) fn cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join(CACHE_FILE))
}
