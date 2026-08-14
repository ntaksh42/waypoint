//! 設定ファイルの読み書きと変数展開。
//!
//! 設定は `%APPDATA%\waypoint\config.json` に置く単一 JSON。
//! 手編集を想定するため、項目順を保つ (serde_json の preserve_order) 。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 項目の型。JSON の "type" で判別する。
///
/// `rename_all` は列挙子の名前 (folder / specialFolder …) に効く。
/// フィールド名は別途 `rename_all_fields` で camelCase にする
/// (known_folder → knownFolder)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Item {
    /// 通常のフォルダ。パスは変数を含んでよい。
    Folder {
        name: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        /// Git ブランチ名を表示するか (FR-2.14) 。
        /// 項目ごとのオプトイン。既定の偽では `.git` を一切読まない。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show_branch: bool,
    },
    /// 既知フォルダ (Downloads, Desktop など) 。
    SpecialFolder {
        name: String,
        known_folder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenMode>,
    },
    /// 入れ子のメニュー。
    Submenu {
        name: String,
        #[serde(default)]
        items: Vec<Item>,
    },
    /// 区切り線。name があれば見出しとして描く。
    Separator {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl Item {
    /// メニューに表示する文字列。separator の name なしは None。
    pub fn label(&self) -> Option<&str> {
        match self {
            Item::Folder { name, .. }
            | Item::SpecialFolder { name, .. }
            | Item::Submenu { name, .. } => Some(name),
            Item::Separator { name } => name.as_deref(),
        }
    }
}

/// フォルダの開き方。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum OpenMode {
    /// 新しいエクスプローラーウィンドウで開く。
    #[default]
    NewWindow,
    /// 元ウィンドウのフォルダを変更する。
    Reuse,
}

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
            search_paths: false,
            visible_results: default_visible_results(),
        }
    }
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
    "Ctrl+Shift+Q".to_string()
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

/// 初回起動時に置く既定の設定。空だと何もできないので数件入れておく。
fn default_config() -> Config {
    Config {
        items: vec![
            Item::SpecialFolder {
                name: "Desktop".to_string(),
                known_folder: "Desktop".to_string(),
                open: None,
            },
            Item::SpecialFolder {
                name: "Downloads".to_string(),
                known_folder: "Downloads".to_string(),
                open: None,
            },
            Item::SpecialFolder {
                name: "Documents".to_string(),
                known_folder: "Documents".to_string(),
                open: None,
            },
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
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    if path.exists() {
        // 既存を .bak へ退避してから差し替える
        let bak = path.with_extension("bak.json");
        let _ = std::fs::remove_file(&bak);
        std::fs::rename(path, &bak)?;
    }
    std::fs::rename(&tmp, path)
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
            Item::Folder { name, path, .. } if expand(path, vars).is_none() => {
                found.push((name.clone(), path.clone()));
            }
            Item::Submenu { items, .. } => collect_unresolved(items, vars, found),
            _ => {}
        }
    }
}

/// パス中の `%ENV%` と `{UserVar}` を展開する (FR-5.1 / FR-5.2) 。
///
/// 解決できない変数が残った場合は None を返し、呼び出し側で
/// グレー表示にする (FR-5.4) 。
pub fn expand(path: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let expanded = expand_user_vars(path, vars)?;
    expand_env_vars(&expanded)
}

fn expand_user_vars(input: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('{') {
        let end = rest[start..].find('}')? + start;
        out.push_str(&rest[..start]);
        let key = &rest[start + 1..end];
        out.push_str(vars.get(key)?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn expand_env_vars(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        let end = rest[start + 1..].find('%')? + start + 1;
        out.push_str(&rest[..start]);
        let key = &rest[start + 1..end];
        // %% はリテラルの % として扱う
        if key.is_empty() {
            out.push('%');
        } else {
            out.push_str(&std::env::var(key).ok()?);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}
