use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedWorkItem {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) work_item_type: Option<String>,
    pub(crate) state: Option<String>,
    #[allow(dead_code)]
    pub(crate) assigned_to: Option<String>,
    #[allow(dead_code)]
    pub(crate) assigned_to_unique_name: Option<String>,
    #[allow(dead_code)]
    pub(crate) changed_date: Option<String>,
    pub(crate) web_url: Option<String>,
    #[allow(dead_code)]
    pub(crate) tags: Option<String>,
}

pub(crate) fn read_work_items(
    conn: &Connection,
    organization: &str,
    project: &str,
) -> Result<Vec<SharedWorkItem>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, title, work_item_type, state, assigned_to, assigned_to_unique_name,
                changed_date, web_url, tags
         FROM work_items WHERE organization = ?1 AND project = ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![organization, project], |row| {
            Ok(SharedWorkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                work_item_type: row.get(2)?,
                state: row.get(3)?,
                assigned_to: row.get(4)?,
                assigned_to_unique_name: row.get(5)?,
                changed_date: row.get(6)?,
                web_url: row.get(7)?,
                tags: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Replaces every work item row for `(organization, project)` with `rows`.
pub(crate) fn write_work_items(
    conn: &mut Connection,
    organization: &str,
    project: &str,
    rows: &[SharedWorkItem],
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM work_items WHERE organization = ?1 AND project = ?2",
        params![organization, project],
    )
    .map_err(|error| error.to_string())?;
    {
        let mut statement = tx
            .prepare(
                "INSERT INTO work_items
             (organization, project, id, title, work_item_type, state, assigned_to,
              assigned_to_unique_name, changed_date, web_url, tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(|error| error.to_string())?;
        for row in rows {
            statement
                .execute(params![
                    organization,
                    project,
                    row.id,
                    row.title,
                    row.work_item_type,
                    row.state,
                    row.assigned_to,
                    row.assigned_to_unique_name,
                    row.changed_date,
                    row.web_url,
                    row.tags,
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE work_items (
                organization TEXT NOT NULL, project TEXT NOT NULL, id INTEGER NOT NULL,
                title TEXT NOT NULL, work_item_type TEXT, state TEXT, assigned_to TEXT,
                assigned_to_unique_name TEXT, changed_date TEXT, web_url TEXT, tags TEXT,
                PRIMARY KEY (organization, id)
            );",
        )
        .unwrap();
    }

    fn sample(id: i64) -> SharedWorkItem {
        SharedWorkItem {
            id,
            title: format!("Item {id}"),
            work_item_type: Some("Bug".to_string()),
            state: Some("Active".to_string()),
            assigned_to: Some("Alice".to_string()),
            assigned_to_unique_name: Some("alice@example.com".to_string()),
            changed_date: Some("2026-01-01T00:00:00Z".to_string()),
            web_url: Some(format!(
                "https://dev.azure.com/org/proj/_workitems/edit/{id}"
            )),
            tags: Some("triaged; needs-repro".to_string()),
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        write_work_items(&mut conn, "org", "proj", &[sample(1)]).unwrap();
        assert_eq!(
            read_work_items(&conn, "org", "proj").unwrap(),
            vec![sample(1)]
        );
    }

    #[test]
    fn write_replaces_the_previous_snapshot_for_that_project_only() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        write_work_items(&mut conn, "org", "proj-a", &[sample(1)]).unwrap();
        write_work_items(&mut conn, "org", "proj-b", &[sample(2)]).unwrap();

        write_work_items(&mut conn, "org", "proj-a", &[sample(3)]).unwrap();
        assert_eq!(
            read_work_items(&conn, "org", "proj-a").unwrap(),
            vec![sample(3)]
        );
        assert_eq!(
            read_work_items(&conn, "org", "proj-b").unwrap(),
            vec![sample(2)]
        );
    }
}
