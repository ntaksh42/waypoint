//! `k ` / `cs ` のスナップショット候補。
//!
//! 通常の検索は `Index` が事前計算した `LowerKeys` を使うが、都度取る
//! 一覧 (プロセス・過去セッション) にはそれが無く、打鍵ごとに全候補の
//! 小文字キー (3 × `to_lowercase`) を作り直していた。候補と検索キーを
//! 対にして持ち、取得時に 1 回だけ作る。

use super::Entry;
use super::rank::search_entries_cached;
use super::search::LowerKeys;
use crate::quick_launch_history::Ranking;

pub(crate) struct Snapshot {
    entries: Vec<Entry>,
    lower: Vec<LowerKeys>,
}

impl Snapshot {
    pub(crate) fn new(entries: Vec<Entry>) -> Self {
        let lower = LowerKeys::build_for(&entries);
        Self { entries, lower }
    }

    pub(crate) fn search(&self, text: &str, search_paths: bool, ranking: &Ranking) -> Vec<&Entry> {
        search_entries_cached(&self.entries, &self.lower, text, search_paths, ranking)
    }
}
