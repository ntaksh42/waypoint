//! 別名 (FR-9.30) を検索結果へ反映する。
//!
//! 別名の照合は通常の順位付け (`rank.rs`) の外で行い、出来上がった一覧に
//! 差し込む。別名は数十件規模なので、打鍵ごとに全件を見ても誤差で済む。
//! 索引を引き直すのは別名が一致したときだけ。

use std::collections::HashSet;

use super::{Entry, Index};
use crate::quick_launch_aliases::AliasRecord;
use crate::quick_launch_history::{key_for, key_kind};

impl Index {
    /// `results` (通常の検索結果) に別名の一致を差し込み、`limit` 件に切る。
    ///
    /// 入力全体が別名と一致した候補は先頭に、別名が入力で始まる候補は
    /// 名前が前方一致した候補の直後に置く。どちらも使用頻度順。
    /// 空白を含む入力 (プレフィックス付き・複数語) とパス入力には効かない。
    pub fn apply_aliases(&self, query: &str, mut results: Vec<Entry>, limit: usize) -> Vec<Entry> {
        if self.aliases.is_empty()
            || query.is_empty()
            || query.contains(char::is_whitespace)
            || super::normalize_path_input(query).is_some()
        {
            return results;
        }
        let term = query.to_lowercase();
        let (exact, prefix) = self.aliases.lookup(&term);
        if exact.is_empty() && prefix.is_empty() {
            return results;
        }
        let mut seen = HashSet::new();
        let exact = self.resolve_aliases(&exact, &mut seen);
        let prefix = self.resolve_aliases(&prefix, &mut seen);
        results.retain(|entry| key_for(entry).is_none_or(|key| !seen.contains(&key)));
        let after_name_prefix = results
            .iter()
            .position(|entry| !entry.name.to_lowercase().starts_with(&term))
            .unwrap_or(results.len());
        results.splice(after_name_prefix..after_name_prefix, prefix);
        results.splice(0..0, exact);
        results.truncate(limit);
        results
    }

    /// 別名を候補へ戻す。同じ候補を指す別名は 1 件にまとめ、使用頻度順に並べる。
    /// `seen` には返した候補のキーを足す。
    fn resolve_aliases(&self, records: &[&AliasRecord], seen: &mut HashSet<String>) -> Vec<Entry> {
        let mut entries: Vec<Entry> = records
            .iter()
            .filter(|record| seen.insert(record.key.clone()))
            .filter_map(|record| self.resolve_alias(record))
            .collect();
        entries
            .sort_by_cached_key(|entry| self.ranking.rank_lower(entry, &entry.path.to_lowercase()));
        entries
    }

    /// 索引に同じ候補があればそれを、無ければ保存内容から組み立てた候補を返す。
    /// 索引側を優先するのは、表示名の変更・ブランチ名・Azure の状態を反映するため。
    pub(crate) fn resolve_alias(&self, record: &AliasRecord) -> Option<Entry> {
        let (kind, path) = record.kind_and_path()?;
        let (settings, _, _) = super::windows_setting_entries();
        let same = |entry: &&Entry| {
            key_kind(&entry.action) == Some(kind)
                && if kind == "url" {
                    entry.path == path
                } else {
                    same_path_lower(&entry.path, path)
                }
        };
        self.entries
            .iter()
            .chain(&self.bookmarks)
            .chain(&self.history)
            .chain(&self.apps)
            .chain(&self.terminal_folders)
            .chain(&self.editor_folders)
            .chain(settings)
            .chain(self.azure.iter().map(|indexed| &indexed.entry))
            .chain(&self.azure_work_items)
            .find(same)
            .cloned()
            .or_else(|| record.fallback_entry())
    }
}

/// `path` を小文字化したものが `lower` と等しいか。ASCII のパスでは確保しない。
fn same_path_lower(path: &str, lower: &str) -> bool {
    if path.is_ascii() {
        path.eq_ignore_ascii_case(lower)
    } else {
        path.to_lowercase() == lower
    }
}
