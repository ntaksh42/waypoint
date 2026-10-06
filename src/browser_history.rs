//! Chrome / Edge の閲覧履歴を読む。
//!
//! Chromium の `History` は SQLite DB。ブラウザ起動中は書き込みロックが
//! 掛かっていることが多いため `immutable=1` の URI 接続で読む
//! (`read_profile` 参照)。Quick Launch の索引構築時だけ読み、検索・描画の
//! 経路では触らない。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

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
    scan_profiles(profile_paths())
}

fn scan_profiles(profiles: impl IntoIterator<Item = Profile>) -> Vec<Visit> {
    let mut by_url: HashMap<String, (i64, Visit)> = HashMap::new();
    for profile in profiles {
        let Ok(visits) = read_profile_cached(&profile) else {
            // ブラウザの更新中・ロック中でも Quick Launch 全体は使えるよう、
            // そのブラウザだけを無言でスキップする。
            continue;
        };
        for (last_visit, visit) in visits {
            // URL のパス・クエリは大小文字を区別するので、Windows パスと
            // 同じ小文字化でまとめると別ページの履歴が消えてしまう。
            let key = history_url_key(&visit.url);
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

/// Learn の表示タブ・記事内アンカーは同じ記事としてまとめる。
/// 他サイトのクエリや SPA のフラグメントはページを識別するため保持する。
fn history_url_key(url: &str) -> String {
    // 大半の URL は対象外。パースして組み立て直す前に、ホスト名が
    // 含まれるかだけを確保なしで見る (ホスト名は大文字小文字を区別しない)
    const HOST: &[u8] = b"learn.microsoft.com";
    if !url
        .as_bytes()
        .windows(HOST.len())
        .any(|window| window.eq_ignore_ascii_case(HOST))
    {
        return url.to_string();
    }
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    if parsed.host_str() != Some("learn.microsoft.com") {
        return url.to_string();
    }
    parsed.set_fragment(None);
    let query: Vec<_> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "tabs")
        .map(|pair| (pair.0.into_owned(), pair.1.into_owned()))
        .collect();
    parsed.set_query(None);
    if !query.is_empty() {
        parsed.query_pairs_mut().extend_pairs(query);
    }
    parsed.to_string()
}

/// `History` ごとの更新日時・サイズと、そのときに読み取った結果。
/// `Index::build` は設定の保存のたびに走るが、閲覧履歴は変わっていない
/// ことが大半なので、同じ内容なら SQLite を引き直さない。
type Read = (SystemTime, u64, Vec<(i64, Visit)>);
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Read>>> = LazyLock::new(Mutex::default);

fn read_profile_cached(profile: &Profile) -> rusqlite::Result<Vec<(i64, Visit)>> {
    let stamp = std::fs::metadata(&profile.path)
        .ok()
        .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())));
    let Some((modified, len)) = stamp else {
        return read_profile(profile);
    };
    let cached = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&profile.path)
        .filter(|(at, size, _)| *at == modified && *size == len)
        .map(|(_, _, visits)| visits.clone());
    if let Some(visits) = cached {
        return Ok(visits);
    }
    // 読めなかった結果は記憶しない (ロック中・更新中の一時的な失敗が居座らないように)
    let visits = read_profile(profile)?;
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(profile.path.clone(), (modified, len, visits.clone()));
    Ok(visits)
}

/// ブラウザ起動中は `History` が `History-journal` 付きのトランザクション中に
/// なっていることが多く、通常の読み取り専用オープンは `SQLITE_BUSY` で失敗する。
/// `immutable=1` の URI 接続はロックを一切取らないため、ブラウザが開いたままでも読める
/// (実測: 通常オープンは `database is locked` で毎回失敗し、`h ` 検索が常に 0 件になっていた)。
fn read_profile(profile: &Profile) -> rusqlite::Result<Vec<(i64, Visit)>> {
    let uri = format!(
        "file:{}?immutable=1",
        encode_uri_path(&profile.path.display().to_string())
    );
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    read_connection(&connection, profile.browser)
}

/// SQLite の URI フィルタ (`file:...?query`) はパス中の `?` / `#` を
/// クエリ・フラグメント区切りとして解釈してしまう。ユーザー名やプロファイル
/// フォルダ名にこれらの文字が含まれると `unable to open database file` で
/// 失敗するため、URI の区切りとして意味を持つ文字だけ `%XX` にエスケープする。
/// `%` 自身も先にエスケープしないと、エスケープ後の文字列を誤って再解釈される。
pub fn encode_uri_path(path: &str) -> String {
    path.replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23")
}

pub fn read_connection(
    connection: &Connection,
    browser: &str,
) -> rusqlite::Result<Vec<(i64, Visit)>> {
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
    fn history_merges_learn_article_variants_and_keeps_latest_url() {
        let root = std::env::temp_dir().join(format!(
            "waypoint-history-learn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut profiles = Vec::new();
        for browser in ["Chrome", "Edge"] {
            let path = root.join(browser);
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE urls (title TEXT, url TEXT, last_visit_time INTEGER);
                     INSERT INTO urls VALUES ('Install', 'https://learn.microsoft.com/ja-jp/windows/powertoys/install', 10);
                     INSERT INTO urls VALUES ('Install', 'https://learn.microsoft.com/ja-jp/windows/powertoys/install#tabs=winget%2Cextract-094', 20);
                     INSERT INTO urls VALUES ('Install', 'https://learn.microsoft.com/ja-jp/windows/powertoys/install?tabs=winget%2Cextract-094', 30);
                     INSERT INTO urls VALUES ('Other article', 'https://learn.microsoft.com/ja-jp/windows/powertoys/overview', 5);
                     INSERT INTO urls VALUES ('Version', 'https://learn.microsoft.com/en-us/dotnet/api/example?view=net-8.0&tabs=csharp#examples', 4);
                     INSERT INTO urls VALUES ('Version', 'https://learn.microsoft.com/en-us/dotnet/api/example?tabs=vb&view=net-8.0#examples', 4);
                     INSERT INTO urls VALUES ('Version', 'https://learn.microsoft.com/en-us/dotnet/api/example?view=net-9.0&tabs=csharp#examples', 3);
                     INSERT INTO urls VALUES ('SPA', 'https://example.com/#/one', 2);
                     INSERT INTO urls VALUES ('SPA', 'https://example.com/#/two', 1);
                     INSERT INTO urls VALUES ('Other host', 'https://example.com/?tabs=one', 2);
                     INSERT INTO urls VALUES ('Other host', 'https://example.com/?tabs=two', 1);",
                )
                .unwrap();
            if browser == "Edge" {
                connection
                    .execute_batch("UPDATE urls SET last_visit_time = last_visit_time + 100;")
                    .unwrap();
            }
            profiles.push(Profile { browser, path });
        }
        let visits = scan_profiles(profiles);
        assert_eq!(visits.len(), 8);
        assert_eq!(visits[0].title, "Install");
        assert_eq!(
            visits[0].url,
            "https://learn.microsoft.com/ja-jp/windows/powertoys/install?tabs=winget%2Cextract-094"
        );
        assert!(visits.iter().all(|visit| visit.browser == "Edge"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn history_keeps_case_sensitive_urls_and_merges_exact_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "waypoint-history-case-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let profiles: Vec<_> = ["Chrome", "Edge"]
            .into_iter()
            .enumerate()
            .map(|(index, browser)| {
                let path = root.join(browser);
                let connection = Connection::open(&path).unwrap();
                connection
                    .execute_batch(
                        "CREATE TABLE urls (title TEXT, url TEXT, last_visit_time INTEGER);
                INSERT INTO urls VALUES ('Upper', 'https://example.com/Repo?Key=A', 10);
                INSERT INTO urls VALUES ('Lower', 'https://example.com/repo?Key=a', 20);",
                    )
                    .unwrap();
                if index == 1 {
                    connection
                        .execute_batch("UPDATE urls SET last_visit_time = last_visit_time + 100;")
                        .unwrap();
                }
                Profile { browser, path }
            })
            .collect();
        let visits = scan_profiles(profiles);
        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].url, "https://example.com/repo?Key=a");
        assert_eq!(visits[1].url, "https://example.com/Repo?Key=A");
        assert!(visits.iter().all(|visit| visit.browser == "Edge"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_history_file_is_reused_and_a_changed_one_is_read_again() {
        let root = std::env::temp_dir().join(format!(
            "waypoint-history-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("History");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE urls (title TEXT, url TEXT, last_visit_time INTEGER);
                 INSERT INTO urls VALUES ('One', 'https://example.com/one', 1);",
            )
            .unwrap();
        let profile = || Profile {
            browser: "Edge",
            path: path.clone(),
        };

        assert_eq!(scan_profiles([profile()]).len(), 1);
        assert_eq!(scan_profiles([profile()]).len(), 1);

        connection
            .execute_batch("INSERT INTO urls VALUES ('Two', 'https://example.com/two', 2);")
            .unwrap();
        drop(connection);
        let visits = scan_profiles([profile()]);
        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].title, "Two");
        std::fs::remove_dir_all(root).unwrap();
    }
}
