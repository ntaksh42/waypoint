//! Chrome / Edge 拡張とのタブ一覧連携。
//!
//! タブの取得は拡張側、検索は Quick Launch のメモリ上で行う。常駐部は
//! タブ一覧を永続化せず、Native Messaging host から届いたスナップショットだけを持つ。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA,
};
use windows::core::{PCWSTR, w};

/// Native Messaging host から常駐部へ送る、タブ一覧の `WM_COPYDATA::dwData`。
pub const SNAPSHOT_COPYDATA: usize = 0x5750_5442;
/// 常駐部から Native Messaging host へ送る、タブ前面化要求の `WM_COPYDATA::dwData`。
pub const FOCUS_COPYDATA: usize = 0x5750_5446;
/// Native Messaging host が作る非表示ウィンドウのクラス名。
pub const HOST_WINDOW_CLASS: PCWSTR = w!("WaypointTabHostWindow");
/// Native Messaging と `WM_COPYDATA` の双方で受け入れる最大ペイロードサイズ。
pub const MAX_SNAPSHOT_BYTES: usize = 1_000_000;

const NATIVE_HOST_NAME: &str = "com.ntaksh42.waypoint.tabs";
const EXTENSION_ID: &str = "fllmalpfkkdholloicheiplekihkkjeo";
const CHROME_NATIVE_HOST_KEY: PCWSTR =
    w!(r"Software\Google\Chrome\NativeMessagingHosts\com.ntaksh42.waypoint.tabs");
const EDGE_NATIVE_HOST_KEY: PCWSTR =
    w!(r"Software\Microsoft\Edge\NativeMessagingHosts\com.ntaksh42.waypoint.tabs");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Browser {
    Chrome,
    Edge,
}

impl Browser {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
        }
    }
}

/// 拡張が送る、現在開いている 1 タブ。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: i32,
    pub window_id: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    #[serde(rename = "type")]
    kind: String,
    browser: Browser,
    tabs: Vec<Tab>,
}

/// タブを前面化するため、検索結果に保持する識別子。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabTarget {
    pub browser: Browser,
    pub tab_id: i32,
    pub window_id: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FocusRequest {
    #[serde(rename = "type")]
    kind: &'static str,
    browser: Browser,
    tab_id: i32,
    window_id: i32,
}

/// 拡張からの JSON スナップショットを検証して返す。
///
/// `WM_COPYDATA` は同一デスクトップ上の別プロセスからも送れるため、受信側でも
/// 件数と必須 ID を確認する。URL とタイトルはメモリだけに置き、保存しない。
pub fn parse_snapshot(bytes: &[u8]) -> Option<(Browser, Vec<Tab>)> {
    const MAX_TABS: usize = 5_000;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return None;
    }
    let snapshot: Snapshot = serde_json::from_slice(bytes).ok()?;
    if snapshot.kind != "tabs" || snapshot.tabs.len() > MAX_TABS {
        return None;
    }
    let tabs = snapshot
        .tabs
        .into_iter()
        .filter(|tab| tab.id >= 0 && tab.window_id >= 0)
        .collect();
    Some((snapshot.browser, tabs))
}

/// Native Messaging host を現在のユーザーの Chrome / Edge に登録する。
///
/// host 自体はブラウザが標準入出力付きで起動するため、waypoint からは起動しない。
/// 登録だけを常駐部の起動時に済ませ、PowerShell を実行する手順を不要にする。
pub fn register_native_host() -> std::io::Result<()> {
    let host_exe = std::env::current_exe()?
        .parent()
        .map(|dir| dir.join("waypoint-tab-host.exe"))
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "waypoint-tab-host.exe")
        })?;
    let manifest_path = native_host_manifest_path()?;
    let manifest = native_host_manifest(&host_exe);
    std::fs::create_dir_all(
        manifest_path
            .parent()
            .expect("native host manifest always has a parent"),
    )?;
    std::fs::write(&manifest_path, manifest)?;

    for key_path in [CHROME_NATIVE_HOST_KEY, EDGE_NATIVE_HOST_KEY] {
        set_native_host_registry(key_path, &manifest_path)?;
    }
    Ok(())
}

/// インストーラー完了後に表示する、拡張の導入案内を開く。
pub fn open_setup_guide() -> std::io::Result<()> {
    let guide = setup_guide_path(&std::env::current_exe()?);
    crate::shell::open_shell_item(&guide.to_string_lossy())
}

fn native_host_manifest_path() -> std::io::Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA"))?;
    Ok(local_app_data
        .join("waypoint")
        .join("browser-tabs")
        .join(format!("{NATIVE_HOST_NAME}.json")))
}

fn setup_guide_path(waypoint_exe: &Path) -> PathBuf {
    waypoint_exe
        .parent()
        .expect("waypoint executable always has a parent")
        .join("browser-tabs")
        .join("setup.html")
}

fn native_host_manifest(host_exe: &Path) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Waypoint browser tab bridge",
        "path": host_exe,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    }))
    .expect("native host manifest is serializable")
}

fn set_native_host_registry(key_path: PCWSTR, manifest_path: &Path) -> std::io::Result<()> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(std::io::Error::other)?;
    }

    let path = windows::core::HSTRING::from(manifest_path.to_string_lossy().as_ref());
    let bytes =
        unsafe { std::slice::from_raw_parts(path.as_ptr().cast::<u8>(), (path.len() + 1) * 2) };
    let result = unsafe { RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.ok().map_err(std::io::Error::other)
}

/// 起動中の全 Native Messaging host に前面化要求を送る。
///
/// Chrome と Edge はそれぞれ host プロセスを持ち得る。要求にはブラウザ種別も
/// 入れるため、対象外の拡張は無視する。`WM_COPYDATA` は同期送信なので
/// ペイロードの寿命は呼び出し中だけでよい。停止した host が残っていても
/// 実行操作を固めないよう、各 host への送信は 100ms で打ち切る。
pub fn request_focus(target: &TabTarget) -> bool {
    let Ok(payload) = serde_json::to_vec(&FocusRequest {
        kind: "focus",
        browser: target.browser,
        tab_id: target.tab_id,
        window_id: target.window_id,
    }) else {
        return false;
    };
    let copy_data = COPYDATASTRUCT {
        dwData: FOCUS_COPYDATA,
        cbData: payload.len() as u32,
        lpData: payload.as_ptr().cast_mut().cast(),
    };
    let mut previous = None;
    let mut sent = false;
    loop {
        let Ok(host) = (unsafe { FindWindowExW(None, previous, HOST_WINDOW_CLASS, None) }) else {
            break;
        };
        if host.0.is_null() {
            break;
        }
        let result = unsafe {
            SendMessageTimeoutW(
                host,
                WM_COPYDATA,
                WPARAM(0),
                LPARAM((&copy_data as *const COPYDATASTRUCT).cast::<()>() as isize),
                SMTO_ABORTIFHUNG,
                100,
                None,
            )
        };
        sent |= result.0 != 0;
        previous = Some(host);
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tab_snapshot_without_persisting_extra_fields() {
        let json = br#"{
            "type":"tabs", "browser":"chrome", "tabs":[
                {"id":7,"windowId":3,"title":"Waypoint","url":"https://example.test/","favIconUrl":"https://example.test/icon"}
            ]
        }"#;
        let (browser, tabs) = parse_snapshot(json).unwrap();
        assert_eq!(browser, Browser::Chrome);
        assert_eq!(tabs[0].title, "Waypoint");
        assert_eq!(tabs[0].url, "https://example.test/");
    }

    #[test]
    fn rejects_non_tab_messages_and_invalid_tab_ids() {
        assert!(parse_snapshot(br#"{"type":"focus","browser":"edge","tabs":[]}"#).is_none());
        let (_, tabs) =
            parse_snapshot(br#"{"type":"tabs","browser":"edge","tabs":[{"id":-1,"windowId":1}]}"#)
                .unwrap();
        assert!(tabs.is_empty());
    }

    #[test]
    fn rejects_an_oversized_snapshot_before_json_parsing() {
        assert!(parse_snapshot(&vec![b' '; MAX_SNAPSHOT_BYTES + 1]).is_none());
    }

    #[test]
    fn native_host_manifest_limits_access_to_the_bundled_extension() {
        let manifest = native_host_manifest(Path::new(r"C:\waypoint-tab-host.exe"));
        let json: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(json["name"], NATIVE_HOST_NAME);
        assert_eq!(json["path"], r"C:\waypoint-tab-host.exe");
        assert_eq!(
            json["allowed_origins"],
            serde_json::json!([format!("chrome-extension://{EXTENSION_ID}/")])
        );
    }

    #[test]
    fn setup_guide_is_bundled_next_to_the_extension() {
        assert_eq!(
            setup_guide_path(Path::new(r"C:\Program Files\waypoint\waypoint.exe")),
            PathBuf::from(r"C:\Program Files\waypoint\browser-tabs\setup.html")
        );
    }
}
