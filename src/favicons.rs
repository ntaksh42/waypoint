//! Chrome / Edge の `Favicons` DB から、ブックマーク URL に対応する
//! favicon の画像データ (PNG) を読む。
//!
//! `Bookmarks` (JSON) と同じプロファイルディレクトリに SQLite の
//! `Favicons` が置かれている。ブラウザ起動中は排他ロックされている
//! ことがあるため、読み取り専用・immutable モードで開き、失敗しても
//! 致命的ではないので None を返す。

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// ブックマーク URL に対応する favicon の PNG バイト列を得る。
///
/// `page_url` は完全一致 (Chrome はブックマークした時点の URL をそのまま
/// キーにする)。見つからなければ None。
pub fn lookup(url: &str) -> Option<Vec<u8>> {
    profile_paths()
        .into_iter()
        .find_map(|path| query(&path, url))
}

fn query(db_path: &Path, url: &str) -> Option<Vec<u8>> {
    // 実ファイルを直接開くとブラウザのロックと競合するため immutable で開く。
    // SQLite は URI 中のバックスラッシュを解釈しないのでスラッシュへ変換する。
    let uri = format!(
        "file:/{}?immutable=1",
        db_path.to_string_lossy().replace('\\', "/")
    );
    let conn = Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;

    conn.query_row(
        "SELECT fb.image_data
         FROM icon_mapping im
         JOIN favicon_bitmaps fb ON fb.icon_id = im.icon_id
         WHERE im.page_url = ?1
         ORDER BY fb.width DESC
         LIMIT 1",
        [url],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .ok()
}

fn profile_paths() -> Vec<PathBuf> {
    let Some(local) = dirs::data_local_dir() else {
        return Vec::new();
    };
    [
        local.join("Google\\Chrome\\User Data\\Default\\Favicons"),
        local.join("Microsoft\\Edge\\User Data\\Default\\Favicons"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}
