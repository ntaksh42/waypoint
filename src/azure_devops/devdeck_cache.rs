//! DevDeck (別リポジトリの Tauri 製 Azure DevOps ダッシュボード) の
//! SQLite キャッシュを読み取り専用で直接参照する。
//!
//! waypoint と DevDeck が同じ Azure DevOps 組織を独立にポーリングすると
//! API 呼び出しが重複し、双方の同期が遅くなる。Active PR と Work Item は
//! DevDeck がバックグラウンドで 5 分おきに同期しているので、waypoint 側の
//! 定期同期は行わず、ここで DevDeck の DB を直接読む。DevDeck が起動して
//! いない・DB が無い・スキーマが古い (`created_by_id` 列が無い) 場合でも
//! 空/フォールバック値を返して継続する — DevDeck を落としても waypoint の
//! 他機能を壊してはいけない。
//!
//! Pipeline は DevDeck 側に永続キャッシュが無いので対象外 (Live 検索のみ)。
//! PR の Completed/Abandoned 履歴は DevDeck の対象外 (Active のみ同期) な
//! ので waypoint 自身の `cache.rs` 側キャッシュのまま変えない。

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::config::AzureDevOpsSettings;

use super::Candidate;
use super::Kind;
use super::convert::{project_key, valid_project};

const DEVDECK_DIR: &str = "com.azdodeck.app";
const DEVDECK_DB_FILE: &str = "azdodeck.sqlite3";

fn devdeck_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join(DEVDECK_DIR).join(DEVDECK_DB_FILE))
}

fn open_devdeck_cache() -> Result<Connection, String> {
    let path = devdeck_cache_path().ok_or_else(|| "AppData path is unavailable.".to_string())?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

struct DevDeckPrRow {
    organization: String,
    project_name: String,
    pull_request_id: i64,
    title: String,
    status: String,
    created_by: Option<String>,
    web_url: Option<String>,
    is_mine: bool,
}

/// `created_by_id` 列 (DevDeck の SCHEMA_VERSION 20 以降) を前提にした
/// クエリ。この列が無い古い DevDeck の DB では `prepare` の時点で失敗する
/// ので、呼び出し側は `query_active_pull_requests_without_author_id` へ
/// フォールバックする。
fn query_active_pull_requests(connection: &Connection) -> rusqlite::Result<Vec<DevDeckPrRow>> {
    let mut statement = connection.prepare(
        "SELECT o.name, pr.project_name, pr.pull_request_id, pr.title, pr.status,
                pr.created_by, pr.web_url,
                (pr.created_by_id IS NOT NULL AND pr.created_by_id = o.authenticated_user_id)
                    OR EXISTS(
                        SELECT 1 FROM review_pull_requests r
                        WHERE r.org_id = pr.org_id
                          AND r.repository_id = pr.repository_id
                          AND r.pull_request_id = pr.pull_request_id
                    ) AS is_mine
         FROM pull_requests pr
         JOIN organizations o ON o.id = pr.org_id",
    )?;
    let rows = statement.query_map([], map_pr_row)?;
    rows.collect()
}

/// 古い DevDeck (`created_by_id` 列が無い) 向けのフォールバック。作成者の
/// GUID を持たないので「自分がレビュアーかどうか」だけで is_mine を判定する
/// (誤って false になるだけで、誤って true になることはない)。
fn query_active_pull_requests_without_author_id(
    connection: &Connection,
) -> rusqlite::Result<Vec<DevDeckPrRow>> {
    let mut statement = connection.prepare(
        "SELECT o.name, pr.project_name, pr.pull_request_id, pr.title, pr.status,
                pr.created_by, pr.web_url,
                EXISTS(
                    SELECT 1 FROM review_pull_requests r
                    WHERE r.org_id = pr.org_id
                      AND r.repository_id = pr.repository_id
                      AND r.pull_request_id = pr.pull_request_id
                ) AS is_mine
         FROM pull_requests pr
         JOIN organizations o ON o.id = pr.org_id",
    )?;
    let rows = statement.query_map([], map_pr_row)?;
    rows.collect()
}

fn map_pr_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DevDeckPrRow> {
    Ok(DevDeckPrRow {
        organization: row.get(0)?,
        project_name: row.get(1)?,
        pull_request_id: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        created_by: row.get(5)?,
        web_url: row.get(6)?,
        is_mine: row.get::<_, i64>(7)? != 0,
    })
}

/// DevDeck の `pull_requests` (Active のみ) から Quick Launch 用の候補を
/// 読む。waypoint 自身は Active PR をもう同期しない。
pub(crate) fn active_pull_request_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    if !settings.enabled {
        return Vec::new();
    }
    let Ok(connection) = open_devdeck_cache() else {
        return Vec::new();
    };
    let rows = query_active_pull_requests(&connection)
        .or_else(|_| query_active_pull_requests_without_author_id(&connection))
        .unwrap_or_default();

    let configured: std::collections::HashMap<_, _> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project) && project.include_pull_requests)
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                project,
            )
        })
        .collect();

    rows.into_iter()
        .filter_map(|row| {
            let url = row.web_url?;
            let project = *configured.get(&project_key(&row.organization, &row.project_name))?;
            Some(Candidate {
                kind: Kind::PullRequest,
                name: format!("PR {}: {}", row.pull_request_id, row.title),
                detail: match &row.created_by {
                    Some(author) if !author.is_empty() => format!(
                        "Azure DevOps — {}/{} — {} — by {author}",
                        row.organization, row.project_name, row.status
                    ),
                    _ => format!(
                        "Azure DevOps — {}/{} — {}",
                        row.organization, row.project_name, row.status
                    ),
                },
                status: row.status,
                url,
                organization: row.organization,
                project: row.project_name,
                aliases: project.aliases.clone(),
                priority: project.priority,
                is_mine: row.is_mine,
            })
        })
        .collect()
}

struct DevDeckWorkItemRow {
    organization: String,
    project_name: String,
    id: i64,
    title: String,
    work_item_type: Option<String>,
    state: Option<String>,
    web_url: Option<String>,
}

fn query_work_items(connection: &Connection) -> rusqlite::Result<Vec<DevDeckWorkItemRow>> {
    let mut statement = connection.prepare(
        "SELECT o.name, wi.project_name, wi.id, wi.title, wi.work_item_type, wi.state, wi.web_url
         FROM work_items wi
         JOIN organizations o ON o.id = wi.org_id
         ORDER BY wi.changed_date DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(DevDeckWorkItemRow {
            organization: row.get(0)?,
            project_name: row.get(1)?,
            id: row.get(2)?,
            title: row.get(3)?,
            work_item_type: row.get(4)?,
            state: row.get(5)?,
            web_url: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// DevDeck の `work_items` (直近更新順、バックグラウンド同期対象) から
/// Quick Launch 用の候補を読む。waypoint 自身は Work Item をもう同期しない
/// (ライブ検索 `az wit live` は API を直接叩くので対象外)。
pub(crate) fn work_item_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    if !settings.enabled {
        return Vec::new();
    }
    let Ok(connection) = open_devdeck_cache() else {
        return Vec::new();
    };
    let Ok(rows) = query_work_items(&connection) else {
        return Vec::new();
    };

    let configured: std::collections::HashMap<_, _> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project) && project.include_work_items)
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                project,
            )
        })
        .collect();

    rows.into_iter()
        .filter_map(|row| {
            let url = row.web_url?;
            let project = *configured.get(&project_key(&row.organization, &row.project_name))?;
            let kind = row.work_item_type.as_deref().unwrap_or("Work Item");
            let state = row.state.as_deref().unwrap_or("");
            Some(Candidate {
                kind: Kind::WorkItem,
                status: state.to_string(),
                name: format!("{}: {}", row.id, row.title),
                detail: format!(
                    "Azure DevOps — {}/{} — {kind} {state}",
                    row.organization, row.project_name
                ),
                url,
                organization: row.organization,
                project: row.project_name,
                aliases: project.aliases.clone(),
                priority: project.priority,
                is_mine: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devdeck_schema(connection: &Connection, with_created_by_id: bool) {
        connection
            .execute_batch(
                "CREATE TABLE organizations(
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, authenticated_user_id TEXT
                );
                CREATE TABLE review_pull_requests(
                    org_id TEXT NOT NULL, repository_id TEXT NOT NULL, pull_request_id INTEGER NOT NULL
                );
                CREATE TABLE work_items(
                    org_id TEXT NOT NULL, project_name TEXT NOT NULL, id INTEGER NOT NULL,
                    title TEXT NOT NULL, work_item_type TEXT, state TEXT, changed_date TEXT, web_url TEXT
                );",
            )
            .unwrap();
        let pr_columns = if with_created_by_id {
            "org_id TEXT NOT NULL, project_name TEXT NOT NULL, repository_id TEXT NOT NULL,
             pull_request_id INTEGER NOT NULL, title TEXT NOT NULL, status TEXT NOT NULL,
             created_by TEXT, created_by_id TEXT, web_url TEXT"
        } else {
            "org_id TEXT NOT NULL, project_name TEXT NOT NULL, repository_id TEXT NOT NULL,
             pull_request_id INTEGER NOT NULL, title TEXT NOT NULL, status TEXT NOT NULL,
             created_by TEXT, web_url TEXT"
        };
        connection
            .execute_batch(&format!("CREATE TABLE pull_requests({pr_columns});"))
            .unwrap();
    }

    #[test]
    fn active_pr_query_marks_author_as_mine_when_created_by_id_matches() {
        let connection = Connection::open_in_memory().unwrap();
        devdeck_schema(&connection, true);
        connection
            .execute_batch(
                "INSERT INTO organizations VALUES('org1', 'contoso', 'user-guid-1');
                 INSERT INTO pull_requests VALUES(
                    'org1', 'Proj', 'repo1', 42, 'Add feature', 'active',
                    'Alice', 'user-guid-1', 'https://dev.azure.com/contoso/_git/repo1/pullrequest/42');",
            )
            .unwrap();

        let rows = query_active_pull_requests(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_mine);
    }

    #[test]
    fn active_pr_query_marks_reviewer_as_mine_even_without_authorship() {
        let connection = Connection::open_in_memory().unwrap();
        devdeck_schema(&connection, true);
        connection
            .execute_batch(
                "INSERT INTO organizations VALUES('org1', 'contoso', 'user-guid-1');
                 INSERT INTO pull_requests VALUES(
                    'org1', 'Proj', 'repo1', 42, 'Add feature', 'active',
                    'Bob', 'user-guid-2', 'https://dev.azure.com/contoso/_git/repo1/pullrequest/42');
                 INSERT INTO review_pull_requests VALUES('org1', 'repo1', 42);",
            )
            .unwrap();

        let rows = query_active_pull_requests(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_mine);
    }

    #[test]
    fn active_pr_query_without_author_id_column_falls_back_to_reviewer_only() {
        let connection = Connection::open_in_memory().unwrap();
        devdeck_schema(&connection, false);
        connection
            .execute_batch(
                "INSERT INTO organizations VALUES('org1', 'contoso', 'user-guid-1');
                 INSERT INTO pull_requests VALUES(
                    'org1', 'Proj', 'repo1', 42, 'Add feature', 'active',
                    'Alice', 'https://dev.azure.com/contoso/_git/repo1/pullrequest/42');",
            )
            .unwrap();

        // The created_by_id-aware query fails to prepare against this older
        // schema; the fallback must still succeed with is_mine = false.
        assert!(query_active_pull_requests(&connection).is_err());
        let rows = query_active_pull_requests_without_author_id(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_mine);
    }

    #[test]
    fn active_pull_request_candidates_only_returns_configured_projects() {
        let connection = Connection::open_in_memory().unwrap();
        devdeck_schema(&connection, true);
        connection
            .execute_batch(
                "INSERT INTO organizations VALUES('org1', 'contoso', 'user-guid-1');
                 INSERT INTO pull_requests VALUES(
                    'org1', 'Tracked', 'repo1', 1, 'Tracked PR', 'active',
                    'Alice', 'user-guid-1', 'https://dev.azure.com/contoso/_git/repo1/pullrequest/1');
                 INSERT INTO pull_requests VALUES(
                    'org1', 'Untracked', 'repo2', 2, 'Untracked PR', 'active',
                    'Alice', 'user-guid-1', 'https://dev.azure.com/contoso/_git/repo2/pullrequest/2');",
            )
            .unwrap();
        let rows = query_active_pull_requests(&connection).unwrap();
        let tracked_key = project_key("contoso", "Tracked");
        let matched = rows
            .iter()
            .filter(|row| project_key(&row.organization, &row.project_name) == tracked_key)
            .count();
        assert_eq!(matched, 1);
    }

    #[test]
    fn work_item_query_reads_devdeck_columns_in_expected_order() {
        let connection = Connection::open_in_memory().unwrap();
        devdeck_schema(&connection, true);
        connection
            .execute_batch(
                "INSERT INTO organizations VALUES('org1', 'contoso', NULL);
                 INSERT INTO work_items VALUES(
                    'org1', 'Proj', 7, 'Fix bug', 'Bug', 'Active', '2024-01-01T00:00:00Z',
                    'https://dev.azure.com/contoso/Proj/_workitems/edit/7');",
            )
            .unwrap();
        let rows = query_work_items(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 7);
        assert_eq!(rows[0].title, "Fix bug");
        assert_eq!(rows[0].work_item_type.as_deref(), Some("Bug"));
        assert_eq!(rows[0].state.as_deref(), Some("Active"));
    }
}
