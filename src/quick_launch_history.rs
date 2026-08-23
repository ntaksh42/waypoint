//! Quick Launch で選んだ項目の履歴。使用頻度順の並び替えに使う。
//!
//! `dynamic.rs` の Recent/Frequent Folders 用 history とは別ファイル。
//! あちらは Windows の実ファイルシステム走査が元になっており、
//! Quick Launch 上での選択そのものは記録していない。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::quick_launch::{Action, Entry};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct History {
    entries: HashMap<String, HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    count: u64,
    last_used: u64,
}

fn history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join("quick_launch_history.json"))
}

fn load() -> History {
    let Some(path) = history_path() else {
        return History::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(history: &History) -> std::io::Result<()> {
    let Some(path) = history_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(history).map_err(std::io::Error::other)?;
    crate::config::write_atomic(&path, &text)
}

/// 選択した entry を安定なキーへ変換する。`FocusWindow` は hwnd が
/// プロセスをまたいで安定しないため、履歴の記録対象にしない。
///
/// Windows のパスは大小文字を区別しないため、同じ対象が config 項目と
/// Everything の検索結果など出所の違いで異なる大小文字で現れることが
/// ある (`quick_launch::dedup_by_path` が索引側で行っている正規化と
/// 同じ理由)。小文字化しないと同一対象の使用回数が分裂し、頻度ランキング
/// が正しく積算されない
fn key_for(entry: &Entry) -> Option<String> {
    let kind = match entry.action {
        Action::OpenFolder(_) => "folder",
        Action::OpenUrl(_) => "url",
        Action::OpenWithDefaultHandler => "default",
        Action::LaunchApp => "app",
        Action::FocusWindow(_) | Action::FocusBrowserTab(_) | Action::ReplaceQuery(_) => {
            return None;
        }
    };
    Some(format!("{kind}|{}", entry.path.to_lowercase()))
}

/// 選択を記録する。表示経路とは別に、選んだ後の一手なので同期 I/O で構わない。
pub fn record(entry: &Entry) {
    let Some(key) = key_for(entry) else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());

    let mut history = load();
    let record = history.entries.entry(key).or_insert(HistoryEntry {
        count: 0,
        last_used: 0,
    });
    record.count += 1;
    record.last_used = now;
    let _ = save(&history);
}

/// 使用頻度・最終選択時刻の順位。小さいほど優先してソート先頭に出す。
/// 未選択の項目は最大値になり、常に選択済みの項目より後ろに回る。
#[derive(Debug, Clone, Default)]
pub struct Ranking {
    entries: HashMap<String, HistoryEntry>,
}

impl Ranking {
    pub fn load() -> Self {
        Self {
            entries: load().entries,
        }
    }

    /// (使用回数の少ない順, 最終選択が古い順) のタプル。
    /// 未選択の項目は `(u64::MAX, u64::MAX)` で必ず最後に回る。
    pub fn rank(&self, entry: &Entry) -> (u64, u64) {
        let Some(key) = key_for(entry) else {
            return (u64::MAX, u64::MAX);
        };
        match self.entries.get(&key) {
            Some(record) => (u64::MAX - record.count, u64::MAX - record.last_used),
            None => (u64::MAX, u64::MAX),
        }
    }

    /// テスト専用。ファイル I/O を経由せず、entry を選んだ回数を直接与える。
    #[cfg(test)]
    pub fn with_selection(mut self, entry: &Entry, count: u64, last_used: u64) -> Self {
        if let Some(key) = key_for(entry) {
            self.entries.insert(key, HistoryEntry { count, last_used });
        }
        self
    }
}
