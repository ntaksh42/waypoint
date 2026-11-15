//! Edge のお気に入り編集画面 (FR-6.16, `waypoint-settings.exe --bookmarks`) との連携。
//!
//! 常駐部の役目は、Native Messaging host (`waypoint-bookmark-host.exe`) を Edge に
//! 登録することと、Quick Launch から編集画面を起動することだけ。お気に入りの
//! 読み書きは拡張と編集画面の間で完結し、常駐部の表示・検索経路には入らない。

use std::path::Path;

use windows::core::{PCWSTR, w};

use crate::browser_tabs::{EXTENSION_ID, native_host_manifest_dir, set_native_host_registry};

/// 拡張 (`browser_tabs/extension/bookmarks.js`) が接続する Native Messaging host の名前。
pub const NATIVE_HOST_NAME: &str = "com.ntaksh42.waypoint.bookmarks";
/// 編集画面が待ち受け、host が接続する名前付きパイプ。
pub const PIPE_PATH: &str = r"\\.\pipe\waypoint-bookmarks";
/// 1 フレームの上限。拡張が送るお気に入り全体のスナップショットが収まる大きさにする。
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// 編集画面を開くときに `waypoint-settings.exe` へ渡す引数。
pub const SETTINGS_ARGUMENT: &str = "--bookmarks";

const HOST_EXE: &str = "waypoint-bookmark-host.exe";
// 対象は Edge だけ (FR-6.17)。Chrome には登録しない
const EDGE_NATIVE_HOST_KEY: PCWSTR =
    w!(r"Software\Microsoft\Edge\NativeMessagingHosts\com.ntaksh42.waypoint.bookmarks");

/// Native Messaging host を現在のユーザーの Edge に登録する。
///
/// タブ検索の host (`browser_tabs::register_native_host`) と同じく、常駐部の起動時に
/// manifest を書き、レジストリから参照させる。host の exe が無ければ何もしない。
pub fn register_native_host() -> std::io::Result<()> {
    let host_exe = std::env::current_exe()?
        .parent()
        .map(|dir| dir.join(HOST_EXE))
        .filter(|path| path.is_file())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, HOST_EXE))?;
    let directory = native_host_manifest_dir()?;
    std::fs::create_dir_all(&directory)?;
    let manifest_path = directory.join(format!("{NATIVE_HOST_NAME}.json"));
    std::fs::write(&manifest_path, native_host_manifest(&host_exe))?;
    set_native_host_registry(EDGE_NATIVE_HOST_KEY, &manifest_path)
}

fn native_host_manifest(host_exe: &Path) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Waypoint bookmark editor bridge",
        "path": host_exe,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    }))
    .expect("native host manifest is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_host_manifest_limits_access_to_the_bundled_extension() {
        let manifest = native_host_manifest(Path::new(r"C:\waypoint-bookmark-host.exe"));
        let json: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(json["name"], NATIVE_HOST_NAME);
        assert_eq!(json["path"], r"C:\waypoint-bookmark-host.exe");
        assert_eq!(json["type"], "stdio");
        assert_eq!(
            json["allowed_origins"],
            serde_json::json!([format!("chrome-extension://{EXTENSION_ID}/")])
        );
    }

    #[test]
    fn host_name_matches_the_registry_key() {
        let key = unsafe { EDGE_NATIVE_HOST_KEY.to_string() }.unwrap();
        assert!(key.ends_with(NATIVE_HOST_NAME));
        assert!(key.starts_with(r"Software\Microsoft\Edge\"));
    }
}
