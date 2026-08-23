//! 検索・スコアリング。

use std::cmp::Reverse;

use fuzzy_matcher::FuzzyMatcher;

use super::azure::{AzureCommand, PipelineFilter, azure_command, azure_command_entries};
use super::{
    APPS_PREFIX, AZURE_DEVOPS_PREFIX, BOOKMARK_PREFIX, Entry, FUZZY_MATCHER, HISTORY_PREFIX, Index,
    WINDOW_PREFIX,
};
use crate::quick_launch_history::Ranking;

impl Index {
    /// プレフィックス入力中は、対応する検索対象だけを検索する。
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        if let Some(rest) = query.strip_prefix(BOOKMARK_PREFIX) {
            return search_entries(&self.bookmarks, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(HISTORY_PREFIX) {
            return search_entries(&self.history, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(WINDOW_PREFIX) {
            return search_entries(&self.windows, rest, false, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(APPS_PREFIX) {
            return search_entries(&self.apps, rest, false, &self.ranking);
        }
        if query == AZURE_DEVOPS_PREFIX {
            return azure_command_entries().iter().collect();
        }
        if let Some(command_text) = super::azure::incomplete_azure_command(query) {
            let completions =
                search_entries(azure_command_entries(), command_text, false, &self.ranking);
            if !completions.is_empty() {
                return completions;
            }
        }
        if let Some((command, rest)) = azure_command(query) {
            return match command {
                AzureCommand::All => search_entries(
                    self.azure
                        .iter()
                        .map(|entry| &entry.entry)
                        .chain(self.azure_work_items.iter()),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::PullRequests(filter) => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| {
                            entry.kind == crate::azure_devops::Kind::PullRequest
                                && filter.status.matches(&entry.status)
                                && (!filter.mine || entry.is_mine)
                        })
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::Pipelines(filter) => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| {
                            entry.kind == crate::azure_devops::Kind::Pipeline
                                && match filter {
                                    PipelineFilter::All => true,
                                    PipelineFilter::Definitions => {
                                        entry.status.eq_ignore_ascii_case("definition")
                                    }
                                    PipelineFilter::Failed => {
                                        entry.status.eq_ignore_ascii_case("failed")
                                    }
                                }
                        })
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::Projects => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| entry.kind == crate::azure_devops::Kind::Project)
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::WorkItems => Vec::new(),
            };
        }
        search_entries(&self.entries, query, self.search_paths, &self.ranking)
    }

    /// 絞り込みなし (空クエリ) のときに、Spotlight 風の区分見出し付き一覧を返す。
    /// 区分ごとに使用頻度順の上位 `SECTION_LIMIT` 件だけを載せ、一覧が
    /// 縦に伸びすぎないようにする。空の区分は含めない。
    pub fn sections(&self) -> Vec<(&'static str, Vec<&Entry>)> {
        const SECTION_LIMIT: usize = 6;
        [
            ("Folders", &self.entries),
            ("Open Windows", &self.windows),
            ("Bookmarks", &self.bookmarks),
            ("History", &self.history),
            ("Apps", &self.apps),
        ]
        .into_iter()
        .filter_map(|(label, source)| {
            let top = search_entries(source, "", self.search_paths, &self.ranking)
                .into_iter()
                .take(SECTION_LIMIT)
                .collect::<Vec<_>>();
            (!top.is_empty()).then_some((label, top))
        })
        .collect()
    }

    /// Work Item キャッシュは Index 構築時に読み込み済み。Quick Launch の
    /// キー入力経路では SQLite に触れず、ここで即時に候補の有無を判定する。
    pub fn search_cached_work_items(&self, query: &str) -> Vec<&Entry> {
        search_entries(&self.azure_work_items, query, true, &self.ranking)
    }

    /// ライブ API の結果をメモリ上のキャッシュにも反映する。
    pub fn merge_cached_work_items(&mut self, entries: &[Entry]) {
        for entry in entries {
            if !self
                .azure_work_items
                .iter()
                .any(|cached| cached.path == entry.path)
            {
                self.azure_work_items.push(entry.clone());
            }
        }
    }
}

/// 検索一致の質、Fuzzy スコア、使用履歴、元の順序と候補本体。
type SearchMatch<'a> = (u8, i64, (u64, u64), usize, &'a Entry);

pub(crate) fn search_entries<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matches: Vec<SearchMatch<'a>> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(order, entry)| {
            let name = entry.name.to_lowercase();
            let breadcrumb = entry.breadcrumb.to_lowercase();
            let path = search_paths.then(|| entry.path.to_lowercase());
            let scores: Option<Vec<(u8, i64)>> = terms
                .iter()
                .map(|term| match_score(&name, &breadcrumb, path.as_deref(), term))
                .collect();
            scores.map(|scores| {
                let (tier, fuzzy_score) = scores
                    .into_iter()
                    // 複数語では一番弱い一致を順位に使う。
                    .max_by_key(|(tier, fuzzy_score)| (*tier, Reverse(*fuzzy_score)))
                    .unwrap_or((0, 0));
                (tier, fuzzy_score, ranking.rank(entry), order, entry)
            })
        })
        .collect();
    // 文字列一致の質を最優先し、同点内では使用頻度・最近使った順で並べる
    matches.sort_by_key(|(tier, fuzzy_score, usage, order, _)| {
        (*tier, Reverse(*fuzzy_score), *usage, *order)
    });
    matches
        .into_iter()
        .map(|(_, _, _, _, entry)| entry)
        .collect()
}

/// 同じパスを指す項目 (config の Folder / Recent Folders / Frequent
/// Folders など、出所違いで同一フォルダが複数登録され得る) を 1 件へ
/// たたむ。先に追加された方を残すので、config.items を優先し
/// Recent > Frequent の順にフォールバックする (呼び出し側の追加順)。
pub(crate) fn dedup_by_path(entries: Vec<Entry>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|entry| {
            if entry.path.is_empty() {
                return true;
            }
            seen.insert(entry.path.to_lowercase())
        })
        .collect()
}

fn match_score(name: &str, breadcrumb: &str, path: Option<&str>, term: &str) -> Option<(u8, i64)> {
    if name == term {
        Some((0, 0))
    } else if name.starts_with(term) {
        Some((1, 0))
    } else if name
        .match_indices(term)
        .any(|(index, _)| index == 0 || name[..index].ends_with([' ', '-', '_', '.']))
    {
        Some((2, 0))
    } else if name.contains(term) {
        Some((3, 0))
    } else if breadcrumb.contains(term) {
        Some((4, 0))
    } else if path.is_some_and(|path| path.contains(term)) {
        Some((5, 0))
    } else if let Some(score) = FUZZY_MATCHER.fuzzy_match(name, term) {
        Some((6, score))
    } else if let Some(score) = FUZZY_MATCHER.fuzzy_match(breadcrumb, term) {
        Some((7, score))
    } else if let Some(score) = path.and_then(|path| FUZZY_MATCHER.fuzzy_match(path, term)) {
        Some((8, score))
    } else if crate::romaji::kana_name_matches(name, term) {
        Some((9, 0))
    } else {
        None
    }
}
