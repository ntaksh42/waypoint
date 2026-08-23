//! 設定ファイルの読み書きと変数展開。
//!
//! 設定は `%APPDATA%\waypoint\config.json` に置く単一 JSON。
//! 手編集を想定するため、項目順を保つ (serde_json の preserve_order) 。

mod expand;
mod item;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use expand::expand;
pub use item::{Item, OpenMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSettings {
    #[serde(default = "default_true")]
    pub middle_click: bool,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_excluded")]
    pub excluded_processes: Vec<String>,
}

impl Default for TriggerSettings {
    fn default() -> Self {
        Self {
            middle_click: true,
            hotkey: default_hotkey(),
            excluded_processes: default_excluded(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSettings {
    #[serde(default = "default_icon_size")]
    pub icon_size: u32,
    #[serde(default = "default_true")]
    pub numeric_accelerators: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickLaunchSettings {
    #[serde(default = "default_quick_launch_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_true")]
    pub include_recent_folders: bool,
    #[serde(default = "default_true")]
    pub include_frequent_folders: bool,
    #[serde(default = "default_true")]
    pub include_open_windows: bool,
    #[serde(default = "default_true")]
    pub include_bookmarks: bool,
    #[serde(default = "default_true")]
    pub include_browser_history: bool,
    #[serde(default = "default_true")]
    pub include_apps: bool,
    #[serde(default)]
    pub azure_devops: AzureDevOpsSettings,
    /// Everything 連携 (FR-9.16)。PC 全体のファイル名検索という重い操作
    /// のため、他のプレフィックス機能と違って既定はオフ
    #[serde(default)]
    pub include_everything: bool,
    #[serde(default)]
    pub search_paths: bool,
    #[serde(default = "default_visible_results")]
    pub visible_results: usize,
}

impl Default for QuickLaunchSettings {
    fn default() -> Self {
        Self {
            hotkey: default_quick_launch_hotkey(),
            include_recent_folders: true,
            include_frequent_folders: true,
            include_open_windows: true,
            include_bookmarks: true,
            include_browser_history: true,
            include_apps: true,
            azure_devops: AzureDevOpsSettings::default(),
            include_everything: false,
            search_paths: false,
            visible_results: default_visible_results(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub projects: Vec<AzureDevOpsProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsProject {
    pub organization: String,
    pub project: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub priority: u32,
    /// PR を同期・検索対象にするか。既存設定との互換性のため既定は有効。
    #[serde(default = "default_true")]
    pub include_pull_requests: bool,
    /// Pipeline を同期・検索対象にするか。既存設定との互換性のため既定は有効。
    #[serde(default = "default_true")]
    pub include_pipelines: bool,
    /// Work Item のオンライン検索対象にするか。既存設定との互換性のため既定は有効。
    #[serde(default = "default_true")]
    pub include_work_items: bool,
    /// 興味のある Area Path (WIQL の `UNDER` 条件に使う)。空なら絞り込まずプロジェクト全体を対象にする。
    #[serde(default)]
    pub interest_areas: Vec<String>,
}

impl Default for MenuSettings {
    fn default() -> Self {
        Self {
            icon_size: default_icon_size(),
            numeric_accelerators: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub trigger: TriggerSettings,
    #[serde(default)]
    pub menu: MenuSettings,
    #[serde(default)]
    pub quick_launch: QuickLaunchSettings,
    #[serde(default)]
    pub start_with_windows: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub items: Vec<Item>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: default_version(),
            variables: BTreeMap::new(),
            settings: Settings::default(),
            items: Vec::new(),
        }
    }
}

impl Config {
    /// ルートメニューの末尾へ項目を追加する。同じパス (大文字小文字を
    /// 区別しない) を持つ項目がメニュー階層のどこかに既にあれば何もしない
    /// (Quick Launch からの登録で重複を作らないため)。
    /// 追加した場合は true を返す。
    pub fn add_item_if_new(&mut self, item: Item) -> bool {
        if item_path(&item).is_some_and(|path| {
            let path = expand(path, &self.variables).unwrap_or_else(|| path.to_string());
            contains_path(&self.items, &path, &self.variables)
        }) {
            return false;
        }
        self.items.push(item);
        true
    }
}

fn item_path(item: &Item) -> Option<&str> {
    match item {
        Item::Folder { path, .. } | Item::File { path, .. } => Some(path),
        _ => None,
    }
}

fn contains_path(items: &[Item], path: &str, variables: &BTreeMap<String, String>) -> bool {
    items.iter().any(|item| match item {
        Item::Folder { path: p, .. } | Item::File { path: p, .. } => expand(p, variables)
            .unwrap_or_else(|| p.clone())
            .eq_ignore_ascii_case(path),
        Item::Submenu { items, .. } => contains_path(items, path, variables),
        _ => false,
    })
}

fn default_true() -> bool {
    true
}
fn default_version() -> u32 {
    1
}
/// QAP の既定と同じ 32px。16px はメニューが窮屈で視認性が落ちる
fn default_icon_size() -> u32 {
    32
}

fn default_quick_launch_hotkey() -> String {
    "Alt+Space".to_string()
}

fn default_visible_results() -> usize {
    12
}
/// 既定のホットキー。
///
/// 仕様書 (`docs/spec.md` FR-1.2) 通り `Win+W`。ただし Windows 11 では
/// Widgets が予約済みで `RegisterHotKey` が「既に登録されています」で
/// 失敗する環境がある (`Win+Q` も検索に予約済み、実測で確認済み)。
/// その場合は低レベルキーボードフックで横取りする (FR-1.2.1)。
fn default_hotkey() -> String {
    "Win+W".to_string()
}
fn default_excluded() -> Vec<String> {
    vec![
        "chrome.exe".to_string(),
        "msedge.exe".to_string(),
        "firefox.exe".to_string(),
    ]
}

/// 設定ファイルのパス。`%APPDATA%\waypoint\config.json`
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("waypoint").join("config.json"))
}

/// 読み込み結果。パース失敗でもアプリは起動する (FR-7.4) 。
pub enum LoadOutcome {
    Loaded(Config),
    /// 設定ファイルが無いので既定を生成した。
    Created(Config),
    /// パースに失敗した。空メニューで起動し、元ファイルは上書きしない。
    Failed(String),
}

/// 設定を読み込む。無ければ既定を書き出す (FR-7.2) 。
pub fn load() -> LoadOutcome {
    let Some(path) = config_path() else {
        return LoadOutcome::Failed("APPDATA を解決できない".to_string());
    };

    if !path.exists() {
        // `write_atomic` は「.bak へ退避 → tmp を本配置へ rename」の 2 段階。
        // 前者が終わった直後にクラッシュ/電源断すると config.json が消え、
        // ここで無条件にデフォルトへ再シードするとユーザー設定が丸ごと
        // 失われる。.bak.json が残っていればまずそちらを試す
        let bak = path.with_extension("bak.json");
        if let Ok(text) = std::fs::read_to_string(&bak)
            && let Ok(cfg) = serde_json::from_str::<Config>(&text)
        {
            crate::panic_log::record(
                "config.json が見つからず .bak.json から復元した (中断された保存の可能性)",
            );
            return LoadOutcome::Loaded(cfg);
        }
        let cfg = default_config();
        if let Err(e) = save(&cfg) {
            return LoadOutcome::Failed(format!("既定設定の書き出しに失敗: {e}"));
        }
        return LoadOutcome::Created(cfg);
    }

    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => LoadOutcome::Loaded(cfg),
            // 壊れた設定を上書きしないよう、ここでは書き込まない (FR-7.4)
            Err(e) => LoadOutcome::Failed(format!("config.json の解析に失敗: {e}")),
        },
        Err(e) => LoadOutcome::Failed(format!("config.json を読めない: {e}")),
    }
}

/// QAP と同じ構成の「My Special Folders」サブメニュー。
///
/// 初回起動時の既定設定と、設定 UI からのプリセット追加 (FR-6) の
/// 両方から使うため、項目の並びをここへ集約する。
pub fn my_special_folders_item() -> Item {
    Item::Submenu {
        name: "My Special Folders".to_string(),
        items: vec![
            Item::SpecialFolder {
                name: "Desktop".to_string(),
                known_folder: "Desktop".to_string(),
                open: None,
            },
            Item::SpecialFolder {
                name: "Documents".to_string(),
                known_folder: "Documents".to_string(),
                open: None,
            },
            Item::SpecialFolder {
                name: "Pictures".to_string(),
                known_folder: "Pictures".to_string(),
                open: None,
            },
            Item::SpecialFolder {
                name: "Downloads".to_string(),
                known_folder: "Downloads".to_string(),
                open: None,
            },
            Item::Separator { name: None },
            Item::Shell {
                name: "This PC".to_string(),
                target: "shell:MyComputerFolder".to_string(),
            },
            Item::Shell {
                name: "Network".to_string(),
                target: "shell:NetworkPlacesFolder".to_string(),
            },
            Item::Shell {
                name: "All Control Panel Items".to_string(),
                target: "shell:ControlPanelFolder".to_string(),
            },
            Item::Shell {
                name: "Recycle Bin".to_string(),
                target: "shell:RecycleBinFolder".to_string(),
            },
        ],
        show_branch: false,
    }
}

/// 初回起動時に置く既定の設定。空だと何もできないので数件入れておく。
fn default_config() -> Config {
    Config {
        items: vec![
            my_special_folders_item(),
            Item::Separator { name: None },
            Item::Folder {
                name: "Profile".to_string(),
                path: "%USERPROFILE%".to_string(),
                open: None,
                icon: None,
                show_branch: false,
            },
        ],
        ..Default::default()
    }
}

/// 設定を保存する。temp に書いてから置換し、途中状態を残さない (FR-7.3) 。
pub fn save(cfg: &Config) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Err(std::io::Error::other("APPDATA を解決できない"));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 手編集するので整形して書く。日本語はエスケープされない (serde_json の既定)
    let text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    write_atomic(&path, &text)
}

/// クラッシュや電源断でファイルが途中状態にならないよう、
/// temp に書いてから置換する。書き込み中の中断では既存ファイルが無傷で残る。
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    if path.exists() {
        // ReplaceFileW なら既存ファイルを残したまま原子的に差し替えられる。
        // std::fs::rename(tmp, path) は Windows では既存 path を上書きできない
        let bak = path.with_extension("bak.json");
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};
        use windows::core::PCWSTR;

        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let tmp_wide: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
        let bak_wide: Vec<u16> = bak.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            ReplaceFileW(
                PCWSTR(path_wide.as_ptr()),
                PCWSTR(tmp_wide.as_ptr()),
                PCWSTR(bak_wide.as_ptr()),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
            .map_err(std::io::Error::other)?;
        }
        Ok(())
    } else {
        std::fs::rename(&tmp, path)
    }
}

/// 変数を解決できない項目を洗い出す (FR-5.4) 。
///
/// 返すのは (項目名, 展開前のパス) 。メニューではグレー表示になるだけで
/// 理由が分からないため、起動時にログへ残すのに使う。
pub fn unresolved_items(config: &Config) -> Vec<(String, String)> {
    let mut found = Vec::new();
    collect_unresolved(&config.items, &config.variables, &mut found);
    found
}

fn collect_unresolved(
    items: &[Item],
    vars: &BTreeMap<String, String>,
    found: &mut Vec<(String, String)>,
) {
    for item in items {
        match item {
            Item::Folder { name, path, .. } | Item::File { name, path, .. }
                if expand(path, vars).is_none() =>
            {
                found.push((name.clone(), path.clone()));
            }
            Item::Submenu { items, .. } => collect_unresolved(items, vars, found),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[test]
    fn atomic_write_replaces_an_existing_file() {
        let root = std::env::temp_dir().join(format!(
            "waypoint-atomic-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("data.json");

        write_atomic(&path, "first").unwrap();
        write_atomic(&path, "second").unwrap();
        write_atomic(&path, "third").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "third");
        assert_eq!(
            std::fs::read_to_string(root.join("data.bak.json")).unwrap(),
            "second"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
