//! Quick Launch の候補に付ける別名 (エイリアス、FR-9.30)。
//!
//! 別名は `%APPDATA%\waypoint\aliases.json` に置き、`config.json` には入れない。
//! config.json は管理画面 (別プロセス) も丸ごと書き戻すため、Quick Launch から
//! 書き足すと管理画面の保存で消えうる。
//!
//! 候補の同一性は使用履歴と同じキー (`quick_launch_history::key_for`、
//! `"{kind}|{path}"`) で判定する。開いているウィンドウとタブの別名はメモリに保持する。

mod live;

pub(crate) use live::{LiveAliasTarget, LiveKey};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::OpenMode;
use crate::quick_launch::{Action, Entry};
use crate::quick_launch_history::key_for;

/// 保存のたびに進める版数。バックグラウンドで構築した `Index` が保存より前に
/// ファイルを読んでいた場合に、手元の新しい別名を古い内容で上書きしないために使う
/// (`quick_launch_window::install_index`)。
static REVISION: AtomicU64 = AtomicU64::new(0);

/// 別名 1 件。候補が索引に無いときでも開けるよう、付けた時点の内容を持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasRecord {
    /// 小文字化済みの別名。
    pub alias: String,
    /// `quick_launch_history::key_for` と同じ `"{kind}|{path}"`。
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub breadcrumb: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<OpenMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
}

impl AliasRecord {
    /// キーの種別 (`folder` / `url` など) と、照合用のパス。
    pub fn kind_and_path(&self) -> Option<(&str, &str)> {
        self.key.split_once('|')
    }

    /// 索引に同じ候補が無いときに使う、保存内容から組み立てた候補。
    pub fn fallback_entry(&self) -> Option<Entry> {
        let (kind, _) = self.kind_and_path()?;
        let action = match kind {
            "folder" => Action::OpenFolder(self.open.unwrap_or_default()),
            "url" => Action::OpenUrl(self.path.clone()),
            "default" => Action::OpenWithDefaultHandler,
            "app" => Action::LaunchApp,
            "setting" => Action::OpenWindowsSetting,
            "terminal" => Action::OpenInTerminal,
            "editor" => Action::OpenInEditor(self.editor.clone().unwrap_or_else(|| "code".into())),
            _ => return None,
        };
        Some(Entry {
            name: self.name.clone(),
            breadcrumb: self.breadcrumb.clone(),
            path: self.path.clone(),
            action,
            branch: None,
            azure: None,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Aliases {
    #[serde(default)]
    aliases: Vec<AliasRecord>,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    pub(crate) live: live::LiveAliases,
}

fn aliases_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join("aliases.json"))
}

impl Aliases {
    /// ファイルから読む。無い・読めないときは空。
    pub fn load() -> Self {
        // 読む前の版数を持つ。読んでいる間に保存が走れば、こちらが古いと分かる
        let revision = REVISION.load(Ordering::SeqCst);
        let mut aliases: Self = aliases_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        aliases.revision = revision;
        aliases
    }

    /// 同期で保存する。別名を確定したときだけ呼ばれ、表示経路には載らない。
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = aliases_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::config::write_atomic(&path, &text)?;
        self.revision = REVISION.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty() && self.live.is_empty()
    }

    pub fn records(&self) -> &[AliasRecord] {
        &self.aliases
    }

    /// 別名を付けられる候補か。
    pub fn supports(entry: &Entry) -> bool {
        live::supports(entry) || key_for(entry).is_some()
    }

    /// `entry` に付いている別名 (付けた順)。
    pub fn aliases_of(&self, entry: &Entry) -> Vec<&str> {
        let Some(key) = key_for(entry) else {
            return Vec::new();
        };
        self.aliases
            .iter()
            .filter(|record| record.key == key)
            .map(|record| record.alias.as_str())
            .collect()
    }

    /// `entry` の別名を `aliases` で置き換える。空なら外す。
    /// 付けられない候補なら何もせず `false` を返す。
    pub fn set(&mut self, entry: &Entry, aliases: Vec<String>) -> bool {
        let Some(key) = key_for(entry) else {
            return false;
        };
        self.aliases.retain(|record| record.key != key);
        let (open, editor) = match &entry.action {
            Action::OpenFolder(mode) => (Some(*mode), None),
            Action::OpenInEditor(command) => (None, Some(command.clone())),
            _ => (None, None),
        };
        self.aliases
            .extend(aliases.into_iter().map(|alias| AliasRecord {
                alias,
                key: key.clone(),
                name: entry.name.clone(),
                breadcrumb: entry.breadcrumb.clone(),
                path: entry.path.clone(),
                open,
                editor: editor.clone(),
            }));
        true
    }

    /// 小文字化済みの `term` に対し、完全一致と前方一致 (完全一致を除く) の別名を返す。
    pub fn lookup(&self, term: &str) -> (Vec<&AliasRecord>, Vec<&AliasRecord>) {
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        for record in &self.aliases {
            if record.alias == term {
                exact.push(record);
            } else if record.alias.starts_with(term) {
                prefix.push(record);
            }
        }
        (exact, prefix)
    }

    /// 他の版の別名を引き継ぐ (`install_index` で新しい方を残すため)。
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.revision > other.revision
    }
}

/// 入力欄の文字列を別名の一覧にする。空白区切り・小文字化・重複除去。
/// 使えない別名があればその別名を添えたエラー文を返す。
pub fn parse(input: &str) -> Result<Vec<String>, String> {
    let mut aliases: Vec<String> = Vec::new();
    for alias in input.split_whitespace() {
        let alias = alias.to_lowercase();
        // パス入力 (FR-9.24) と `??` (FR-9.21) に重なる別名は照合されない
        if alias.contains(['\\', '/', ':']) || alias.starts_with('?') {
            return Err(format!(
                "Cannot use \"{alias}\": aliases must not contain \\ / : or start with ?"
            ));
        }
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    Ok(aliases)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str) -> Entry {
        Entry {
            name: "waypoint".into(),
            breadcrumb: "Dev".into(),
            path: path.into(),
            action: Action::OpenFolder(OpenMode::Reuse),
            branch: None,
            azure: None,
        }
    }

    #[test]
    fn parse_lowercases_and_dedups() {
        assert_eq!(parse(" WP  way wp ").unwrap(), vec!["wp", "way"]);
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn parse_rejects_path_like_and_question_mark() {
        for bad in ["c:", "a/b", r"a\b", "?x", "??"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn set_replaces_aliases_of_same_target_regardless_of_case() {
        let mut aliases = Aliases::default();
        assert!(aliases.set(&folder(r"E:\waypoint"), vec!["wp".into(), "way".into()]));
        assert!(aliases.set(&folder(r"e:\WAYPOINT"), vec!["w".into()]));
        assert_eq!(aliases.aliases_of(&folder(r"E:\waypoint")), vec!["w"]);
        aliases.set(&folder(r"E:\waypoint"), Vec::new());
        assert!(aliases.is_empty());
    }

    #[test]
    fn set_ignores_candidates_without_stable_key() {
        let mut aliases = Aliases::default();
        let window = Entry {
            action: Action::FocusWindow(1),
            ..folder("x")
        };
        assert!(!aliases.set(&window, vec!["x".into()]));
        assert!(aliases.is_empty());
    }

    #[test]
    fn lookup_splits_exact_and_prefix() {
        let mut aliases = Aliases::default();
        aliases.set(&folder(r"E:\a"), vec!["wp".into()]);
        aliases.set(&folder(r"E:\b"), vec!["wpx".into()]);
        aliases.set(&folder(r"E:\c"), vec!["x".into()]);
        let (exact, prefix) = aliases.lookup("wp");
        assert_eq!(
            exact.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            [r"E:\a"]
        );
        assert_eq!(
            prefix.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            [r"E:\b"]
        );
    }

    #[test]
    fn fallback_entry_restores_action_and_round_trips_json() {
        let mut aliases = Aliases::default();
        aliases.set(&folder(r"E:\waypoint"), vec!["wp".into()]);
        let text = serde_json::to_string(&aliases).unwrap();
        let loaded: Aliases = serde_json::from_str(&text).unwrap();
        let entry = loaded.records()[0].fallback_entry().unwrap();
        assert_eq!(entry.action, Action::OpenFolder(OpenMode::Reuse));
        assert_eq!(entry.path, r"E:\waypoint");
        assert_eq!(entry.breadcrumb, "Dev");
    }
}
