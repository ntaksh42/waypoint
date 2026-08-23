//! Chrome / Edge の閲覧履歴を読む。
//!
//! Chromium の `History` は SQLite DB。ブラウザ起動中は書き込みロックが
//! 掛かっていることが多いため `immutable=1` の URI 接続で読む
//! (`read_profile` 参照)。Quick Launch の索引構築時だけ読み、検索・描画の
//! 経路では触らない。

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};

const HISTORY_LIMIT_PER_BROWSER: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub title: String,
    pub url: String,
    pub browser: String,
}

struct Profile {
    browser: &'static str,
    path: PathBuf,
}

/// Chrome / Edge の既定プロファイルから、最新順の URL を集める。
pub fn scan() -> Vec<Visit> {
    let mut by_url: HashMap<String, (i64, Visit)> = HashMap::new();
    for profile in profile_paths() {
        let Ok(visits) = read_profile(&profile) else {
            // ブラウザの更新中・ロック中でも Quick Launch 全体は使えるよう、
            // そのブラウザだけを無言でスキップする。
            continue;
        };
        for (last_visit, visit) in visits {
            let key = visit.url.to_lowercase();
            if by_url
                .get(&key)
                .is_none_or(|(known, _)| *known < last_visit)
            {
                by_url.insert(key, (last_visit, visit));
            }
        }
    }

    let mut visits: Vec<_> = by_url.into_values().collect();
    visits.sort_by_key(|(last_visit, _)| std::cmp::Reverse(*last_visit));
    visits.into_iter().map(|(_, visit)| visit).collect()
}

/// ブラウザ起動中は `History` が `History-journal` 付きのトランザクション中に
/// なっていることが多く、通常の読み取り専用オープンは `SQLITE_BUSY` で失敗する。
/// `immutable=1` の URI 接続はロックを一切取らないため、ブラウザが開いたままでも読める
/// (実測: 通常オープンは `database is locked` で毎回失敗し、`h ` 検索が常に 0 件になっていた)。
fn read_profile(profile: &Profile) -> rusqlite::Result<Vec<(i64, Visit)>> {
    let uri = format!("file:{}?immutable=1", profile.path.display());
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
    )?;
    read_connection(&connection, profile.browser)
}

fn read_connection(connection: &Connection, browser: &str) -> rusqlite::Result<Vec<(i64, Visit)>> {
    let mut statement = connection.prepare(
        "SELECT title, url, last_visit_time
         FROM urls
         WHERE url LIKE 'http%'
         ORDER BY last_visit_time DESC
         LIMIT ?1",
    )?;
    let rows = statement.query_map([HISTORY_LIMIT_PER_BROWSER], |row| {
        let title: String = row.get(0)?;
        let url: String = row.get(1)?;
        let last_visit: i64 = row.get(2)?;
        Ok((
            last_visit,
            Visit {
                title: if title.trim().is_empty() {
                    url.clone()
                } else {
                    title
                },
                url,
                browser: browser.to_string(),
            },
        ))
    })?;
    rows.collect()
}

fn profile_paths() -> Vec<Profile> {
    let Some(local) = dirs::data_local_dir() else {
        return Vec::new();
    };
    [
        Profile {
            browser: "Chrome",
            path: local.join("Google\\Chrome\\User Data\\Default\\History"),
        },
        Profile {
            browser: "Edge",
            path: local.join("Microsoft\\Edge\\User Data\\Default\\History"),
        },
    ]
    .into_iter()
    .filter(|profile| profile.path.exists())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_http_urls_and_uses_url_when_title_is_empty() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE urls (title TEXT NOT NULL, url TEXT NOT NULL, last_visit_time INTEGER NOT NULL);
                 INSERT INTO urls VALUES ('WayPoint', 'https://example.com/waypoint', 20);
                 INSERT INTO urls VALUES ('', 'https://example.com/no-title', 10);
                 INSERT INTO urls VALUES ('New tab', 'chrome://newtab/', 30);",
            )
            .unwrap();

        let visits: Vec<_> = read_connection(&connection, "Chrome")
            .unwrap()
            .into_iter()
            .map(|(_, visit)| visit)
            .collect();

        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].title, "WayPoint");
        assert_eq!(visits[1].title, "https://example.com/no-title");
    }
}
