//! Chrome / Edge の閲覧履歴読み込みのテスト。

use rusqlite::Connection;

use waypoint::browser_history::{encode_uri_path, read_connection};

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

/// プロファイルパスに `#` / `?` / `%` が含まれると、SQLite の URI フィルタが
/// クエリ・フラグメント区切りと誤認して `immutable=1` 接続に失敗していた。
#[test]
fn encode_uri_path_escapes_uri_delimiters() {
    assert_eq!(
        encode_uri_path(r"C:\Users\alice#1\AppData\Local\Google\Chrome\User Data\Default\History"),
        r"C:\Users\alice%231\AppData\Local\Google\Chrome\User Data\Default\History"
    );
    assert_eq!(encode_uri_path("100%done"), "100%25done");
    assert_eq!(
        encode_uri_path(r"C:\normal path\History"),
        r"C:\normal path\History"
    );
}
