//! Neutral, cross-app shared cache for Azure DevOps Active PR / Work Item
//! data.
//!
//! DevDeck (a separate, unrelated repository — a Tauri dashboard app) polls
//! the same Azure DevOps organization independently of waypoint's own
//! background sync. Rather than either app depending on the other's private
//! schema, both write into and read from this small, neutral SQLite file at
//! `%APPDATA%\AzDoSharedCache\cache.db`. waypoint does not import DevDeck's
//! crate or types; this module and its counterpart in DevDeck only agree on
//! the table shapes below.
//!
//! Fields here are limited to raw Azure DevOps facts (title, status, the
//! author's identity, reviewers and their votes, ...). "Is this mine" is
//! deliberately not stored here — waypoint computes it itself by comparing
//! `created_by_id` / `reviewer_id` against its own resolved identity
//! (`current_user_id`). Anything specific to DevDeck's own feature set (CI
//! status, merge status, "My Reviews" votes) stays in DevDeck's private
//! cache and is not part of this contract.
//!
//! Freshness is tracked per `(organization, project, kind)` in `sync_state`,
//! separately from the entry rows themselves, so a reader can check "is this
//! fresh enough" without touching the entry tables. How stale a caller is
//! willing to accept is that caller's own policy, not part of this schema.

mod pull_requests;
mod work_items;

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

pub(crate) use pull_requests::{
    SharedPullRequest, SharedReviewer, read_pull_requests, read_reviewers, write_pull_requests,
};
pub(crate) use work_items::{SharedWorkItem, read_work_items, write_work_items};

const DIR_NAME: &str = "AzDoSharedCache";
const FILE_NAME: &str = "cache.db";

/// Identifies who most recently refreshed a scope, for debugging/visibility
/// only (never used in any freshness decision).
pub(crate) const SYNCED_BY: &str = "waypoint";

pub(crate) const KIND_PULL_REQUESTS: &str = "pull_requests";
pub(crate) const KIND_WORK_ITEMS: &str = "work_items";

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(DIR_NAME).join(FILE_NAME))
}

pub(crate) fn open() -> Result<Connection, String> {
    let path = path().ok_or_else(|| "AppData path is unavailable.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS pull_requests (
                organization      TEXT NOT NULL,
                project           TEXT NOT NULL,
                repository_id     TEXT NOT NULL,
                repository_name   TEXT NOT NULL,
                pull_request_id   INTEGER NOT NULL,
                title             TEXT NOT NULL,
                status            TEXT NOT NULL,
                created_by        TEXT,
                created_by_id     TEXT,
                creation_date     TEXT NOT NULL,
                source_ref_name   TEXT NOT NULL,
                target_ref_name   TEXT NOT NULL,
                is_draft          INTEGER NOT NULL DEFAULT 0,
                web_url           TEXT,
                PRIMARY KEY (organization, repository_id, pull_request_id)
            );

            CREATE TABLE IF NOT EXISTS pull_request_reviewers (
                organization     TEXT NOT NULL,
                project          TEXT NOT NULL,
                repository_id    TEXT NOT NULL,
                pull_request_id  INTEGER NOT NULL,
                reviewer_id      TEXT NOT NULL,
                vote             INTEGER NOT NULL,
                is_required      INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (organization, project, repository_id, pull_request_id, reviewer_id)
            );

            CREATE TABLE IF NOT EXISTS work_items (
                organization             TEXT NOT NULL,
                project                  TEXT NOT NULL,
                id                       INTEGER NOT NULL,
                title                    TEXT NOT NULL,
                work_item_type           TEXT,
                state                    TEXT,
                assigned_to              TEXT,
                assigned_to_unique_name  TEXT,
                changed_date             TEXT,
                web_url                  TEXT,
                tags                     TEXT,
                PRIMARY KEY (organization, id)
            );

            CREATE TABLE IF NOT EXISTS sync_state (
                organization  TEXT NOT NULL,
                project       TEXT NOT NULL,
                kind          TEXT NOT NULL,
                synced_at     INTEGER NOT NULL,
                synced_by     TEXT NOT NULL,
                last_error    TEXT,
                PRIMARY KEY (organization, project, kind)
            );

            CREATE TABLE IF NOT EXISTS cache_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO cache_meta (key, value) VALUES ('schema_version', '1');",
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

thread_local! {
    static CACHED_CONNECTION: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

/// Caches the shared-cache connection across repeated reads on the same
/// thread. `cached_candidate_groups` reads this on every Quick Launch index
/// rebuild, not just once, so reopening the file and rerunning the schema
/// setup each time would be wasted work. `operation` reports whether the
/// read it just did was healthy; an unhealthy result (e.g. a query failed
/// because the schema is unexpectedly different) drops the cached connection
/// so the next call opens a fresh one instead of repeating the same failure.
pub(crate) fn with_cached_connection<R>(
    operation: impl FnOnce(&Connection) -> (R, bool),
) -> Result<R, String> {
    CACHED_CONNECTION.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(open()?);
        }
        let (value, healthy) = {
            let connection = cell.borrow();
            operation(
                connection
                    .as_ref()
                    .expect("connection was just initialized"),
            )
        };
        if !healthy {
            cell.borrow_mut().take();
        }
        Ok(value)
    })
}

/// Whether anyone (this app or the other one) refreshed `(organization,
/// project, kind)` within `max_age`. Missing rows are treated as stale.
pub(crate) fn is_fresh(
    conn: &Connection,
    organization: &str,
    project: &str,
    kind: &str,
    max_age: Duration,
) -> bool {
    let synced_at: Option<i64> = conn
        .query_row(
            "SELECT synced_at FROM sync_state WHERE organization = ?1 AND project = ?2 AND kind = ?3",
            params![organization, project, kind],
            |row| row.get(0),
        )
        .ok();
    synced_at.is_some_and(|synced_at| unix_now() - synced_at < max_age.as_secs() as i64)
}

pub(crate) fn mark_synced(
    conn: &Connection,
    organization: &str,
    project: &str,
    kind: &str,
    synced_by: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_state (organization, project, kind, synced_at, synced_by, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT(organization, project, kind) DO UPDATE SET
            synced_at = excluded.synced_at,
            synced_by = excluded.synced_by,
            last_error = NULL",
        params![organization, project, kind, unix_now(), synced_by],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sync_state (
                organization TEXT NOT NULL, project TEXT NOT NULL, kind TEXT NOT NULL,
                synced_at INTEGER NOT NULL, synced_by TEXT NOT NULL, last_error TEXT,
                PRIMARY KEY (organization, project, kind)
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn missing_scope_is_never_fresh() {
        let conn = memory_conn();
        assert!(!is_fresh(
            &conn,
            "org",
            "proj",
            KIND_PULL_REQUESTS,
            Duration::from_secs(600)
        ));
    }

    #[test]
    fn freshly_marked_scope_is_fresh_until_max_age_elapses() {
        let conn = memory_conn();
        mark_synced(&conn, "org", "proj", KIND_PULL_REQUESTS, "devdeck").unwrap();
        assert!(is_fresh(
            &conn,
            "org",
            "proj",
            KIND_PULL_REQUESTS,
            Duration::from_secs(600)
        ));
        assert!(!is_fresh(
            &conn,
            "org",
            "other",
            KIND_PULL_REQUESTS,
            Duration::from_secs(600)
        ));
        assert!(!is_fresh(
            &conn,
            "org",
            "proj",
            KIND_WORK_ITEMS,
            Duration::from_secs(600)
        ));
    }

    #[test]
    fn freshness_check_works_regardless_of_who_synced_it() {
        let conn = memory_conn();
        mark_synced(&conn, "org", "proj", KIND_PULL_REQUESTS, "devdeck").unwrap();
        assert!(is_fresh(
            &conn,
            "org",
            "proj",
            KIND_PULL_REQUESTS,
            Duration::from_secs(600)
        ));
    }
}
