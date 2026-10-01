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

/// 履歴に記録する対象の action か判定し、キーの種別プレフィックスを返す。
/// `FocusWindow` は hwnd がプロセスをまたいで安定しないため対象外。
fn key_kind(action: &Action) -> Option<&'static str> {
    match action {
        Action::OpenFolder(_) => Some("folder"),
        Action::OpenUrl(_) => Some("url"),
        Action::OpenWithDefaultHandler => Some("default"),
        Action::LaunchApp => Some("app"),
        Action::OpenInTerminal => Some("terminal"),
        Action::OpenInEditor(_) => Some("editor"),
        Action::OpenClaudeCode(_) | Action::OpenCodex | Action::ResumeAgentSession(..) => None,
        Action::FocusWindow(_)
        | Action::FocusBrowserTab(_)
        | Action::ReplaceQuery(_)
        | Action::AzureLiveWorkItemSearch(_)
        | Action::AzureLivePullRequestSearch { .. }
        | Action::AzureLivePipelineSearch { .. }
        | Action::AzureOptimize
        | Action::OpenSettings
        | Action::OpenHelp
        // 検索語は毎回異なりうるので記録しない (FR-9.21)
        | Action::WebSearch
        // PID はプロセス再起動のたびに変わるため FocusWindow と同様に対象外
        | Action::KillProcess(_) => None,
    }
}

/// 選択した entry を安定なキーへ変換する。
///
/// Windows のパスは大小文字を区別しないため、同じ対象が config 項目と
/// Everything の検索結果など出所の違いで異なる大小文字で現れることが
/// ある (`quick_launch::dedup_by_path` が索引側で行っている正規化と
/// 同じ理由)。小文字化しないと同一対象の使用回数が分裂し、頻度ランキング
/// が正しく積算されない。URL のパス・クエリは大小文字を区別するため元の綴りを使う。
fn key_for(entry: &Entry) -> Option<String> {
    let kind = key_kind(&entry.action)?;
    let path = if kind == "url" {
        entry.path.clone()
    } else {
        entry.path.to_lowercase()
    };
    Some(format!("{kind}|{path}"))
}

/// JSON 永続化用のキー (`"{kind}|{path}"`) を kind と path に戻す。
/// パスは小文字化済みだが、URL は元の綴りを保持する。
/// 保存側は既存フォーマットのまま (旧バージョンとの互換性)、`Ranking` の
/// 内部表現だけを kind 別の `HashMap` に分けるための変換。
fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('|')
}

/// 選択をバックグラウンドスレッドで記録する。
///
/// 記録は次回以降の並び順にしか影響しないので、書き終わるのを待つ理由が
/// 無い。待つと保存の `REPLACEFILE_WRITE_THROUGH` によるディスク flush
/// (実測 18.8ms) がそのまま「選んでから実際にフォルダ・アプリが開くまで」の
/// 遅延になる (読み込みは 0.049ms で誤差)。
///
/// スレッドは投げっぱなしにする。書き込みが失敗しても次回の記録で
/// 上書きされるだけなので、待って確認する意味が無い。プロセスが直後に
/// 終了した場合は最後の 1 件を落とすが、頻度ランキングの 1 カウントなので
/// 実害が無い。
///
/// 保存は read-modify-write なので、書き込み中に次の記録が始まると
/// 片方のカウント +1 が失われ得る。ただし選択のたびに Quick Launch は
/// 閉じるため、次の記録にはホットキー → 入力 → Enter が要る。書き込みは
/// 20ms 弱で終わるので現実的には競合しない。競合しても失うのは 1 カウント
/// だけで、ファイルが壊れることは無い (`write_atomic` が temp → replace で
/// 差し替えるため、途中状態のファイルは見えない)。ロックを導入するほどの
/// 実害が無いので、この単純さを採る。
pub fn record_async(entry: &Entry) {
    // Entry は呼び出し元スレッドが所有するので、キーだけ取り出して渡す
    let Some(key) = key_for(entry) else {
        return;
    };
    std::thread::spawn(move || record_key(key));
}

/// キー 1 件ぶんの使用回数と最終選択時刻を更新して保存する。
/// 呼び出し元スレッドで同期に I/O する。
fn record_key(key: String) {
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

/// ベンチ用。`record_async` の同期版 (スレッドを挟まず測るため)。
#[cfg(test)]
pub fn record_blocking(entry: &Entry) {
    if let Some(key) = key_for(entry) {
        record_key(key);
    }
}

/// 使用頻度・最終選択時刻の順位。小さいほど優先してソート先頭に出す。
/// 未選択の項目は最大値になり、常に選択済みの項目より後ろに回る。
///
/// `kind` (folder/url/default/app) で外側を分け、内側は `path_lower` を
/// そのままキーにする 2 段構成。検索は候補 1 件につき 1 回、キー入力の
/// たびに全一致候補ぶん呼ばれるため (`quick_launch::search::score_entry`)、
/// フラットな `"{kind}|{path}"` 結合キーだと参照のたびに `format!` の
/// ヒープ確保が発生していた。`path_lower` は呼び出し側で事前計算済みなので、
/// 内側マップを path_lower で直接引ければ確保なしでルックアップできる
/// (1万件規模の候補で実測、空クエリ/プレフィックス検索時に体感差)。
#[derive(Debug, Clone, Default)]
pub struct Ranking {
    entries: HashMap<&'static str, HashMap<String, HistoryEntry>>,
}

impl Ranking {
    pub fn load() -> Self {
        Self::from_history(load())
    }

    fn from_history(history: History) -> Self {
        let mut entries: HashMap<&'static str, HashMap<String, HistoryEntry>> = HashMap::new();
        for (key, record) in history.entries {
            if let Some((kind, path_lower)) = split_key(&key)
                && let Some(kind) = normalize_kind(kind)
            {
                entries
                    .entry(kind)
                    .or_default()
                    .insert(path_lower.to_string(), record);
            }
        }
        Self { entries }
    }

    /// (使用回数の少ない順, 最終選択が古い順) のタプル。
    /// 未選択の項目は `(u64::MAX, u64::MAX)` で必ず最後に回る。
    ///
    /// `path_lower` は呼び出し側が既に小文字化済みの path を渡す。検索の
    /// スコアリング経路 (`quick_launch::search::score_entry`) は候補ごとに
    /// `LowerKeys` で path を事前計算済みのため、ここで
    /// `entry.path.to_lowercase()` を再アロケーションすると事前計算の意味が
    /// 薄れる (候補数百件規模でキー入力のたびに走る経路のため、実測で
    /// 体感できる差になる)。URL の場合は元の `entry.path` で参照する。
    pub fn rank_lower(&self, entry: &Entry, path_lower: &str) -> (u64, u64) {
        let Some(kind) = key_kind(&entry.action) else {
            return (u64::MAX, u64::MAX);
        };
        let path = if kind == "url" {
            &entry.path
        } else {
            path_lower
        };
        match self.entries.get(kind).and_then(|by_path| by_path.get(path)) {
            Some(record) => (u64::MAX - record.count, u64::MAX - record.last_used),
            None => (u64::MAX, u64::MAX),
        }
    }

    /// テスト専用。ファイル I/O を経由せず、entry を選んだ回数を直接与える。
    #[cfg(test)]
    pub fn with_selection(mut self, entry: &Entry, count: u64, last_used: u64) -> Self {
        if let Some(kind) = key_kind(&entry.action) {
            let path = if kind == "url" {
                entry.path.clone()
            } else {
                entry.path.to_lowercase()
            };
            self.entries
                .entry(kind)
                .or_default()
                .insert(path, HistoryEntry { count, last_used });
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_editor_history_survives_reload() {
        for action in [Action::OpenInTerminal, Action::OpenInEditor("code".into())] {
            let entry = Entry {
                name: "Project".into(),
                path: r"E:\Project".into(),
                breadcrumb: String::new(),
                action,
                branch: None,
                azure: None,
            };
            let history = History {
                entries: HashMap::from([(
                    key_for(&entry).unwrap(),
                    HistoryEntry {
                        count: 3,
                        last_used: 42,
                    },
                )]),
            };
            let text = serde_json::to_string(&history).unwrap();
            let ranking = Ranking::from_history(serde_json::from_str(&text).unwrap());
            assert_eq!(
                ranking.rank_lower(&entry, r"e:\project"),
                (u64::MAX - 3, u64::MAX - 42)
            );
        }
    }

    #[test]
    fn url_history_keeps_distinct_case_sensitive_targets() {
        let entry = |path: &str| Entry {
            name: "Page".into(),
            path: path.into(),
            breadcrumb: String::new(),
            action: Action::OpenUrl(path.into()),
            branch: None,
            azure: None,
        };
        let upper = entry("https://example.com/Repo?Key=A");
        let lower = entry("https://example.com/repo?Key=a");
        let history = History {
            entries: HashMap::from([
                (
                    key_for(&upper).unwrap(),
                    HistoryEntry {
                        count: 9,
                        last_used: 42,
                    },
                ),
                (
                    key_for(&lower).unwrap(),
                    HistoryEntry {
                        count: 2,
                        last_used: 41,
                    },
                ),
            ]),
        };
        let ranking = Ranking::from_history(history);
        assert_eq!(
            ranking.rank_lower(&upper, &upper.path.to_lowercase()),
            (u64::MAX - 9, u64::MAX - 42)
        );
        assert_eq!(
            ranking.rank_lower(&lower, &lower.path.to_lowercase()),
            (u64::MAX - 2, u64::MAX - 41)
        );
    }
}

/// `load()` (JSON 経由) が返す kind は `key_kind` と同じ固定文字列のはずだが、
/// 版違いの永続化ファイルで不明な kind が混じっても `&'static str` の
/// 表に正規化してから使う (未知の kind は握りつぶす)。
fn normalize_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "folder" => Some("folder"),
        "url" => Some("url"),
        "default" => Some("default"),
        "app" => Some("app"),
        "terminal" => Some("terminal"),
        "editor" => Some("editor"),
        _ => None,
    }
}

/// ベンチ用。`record` の内訳を測るために load / save を個別に呼ぶ。
#[cfg(test)]
pub mod bench_parts {
    use super::*;

    pub fn load_len() -> usize {
        load().entries.len()
    }

    /// 読み込んだ内容をそのまま保存し直す (更新は挟まない)。
    pub fn save_roundtrip() -> usize {
        let history = load();
        let _ = save(&history);
        history.entries.len()
    }
}
