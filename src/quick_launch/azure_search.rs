//! Azure DevOps 候補専用の検索順位付け。

use std::cmp::Reverse;

use super::Entry;
use super::search::{Fields, Fuzzy, LowerKeys, lower_terms, score_entry};
use crate::quick_launch_history::Ranking;

/// 語句一致と全語タイトル一致を breadcrumb / URL の一致より優先し、
/// 1 文字のタイプミスも拾う。
pub(super) fn search<'a>(
    items: impl IntoIterator<Item = (&'a Entry, &'a LowerKeys)>,
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms = lower_terms(query);
    let mut matches = items
        .into_iter()
        .enumerate()
        .filter_map(|(order, (entry, keys))| {
            let fields = Fields::from_keys(keys, search_paths);
            let title_quality = crate::azure_devops::title_match_quality(&entry.name, query);
            let general = score_entry(entry, fields, &terms, ranking, Fuzzy::Include);
            let (tier, fuzzy_score, usage) = general
                .unwrap_or_else(|| (u8::MAX, 0, ranking.rank_lower(entry, fields.path_lower)));
            title_quality
                .or_else(|| general.map(|_| 3))
                .map(|title_quality| (title_quality, tier, fuzzy_score, usage, order, entry))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(title_quality, tier, fuzzy_score, usage, order, _)| {
        (*title_quality, *tier, Reverse(*fuzzy_score), *usage, *order)
    });
    matches
        .into_iter()
        .map(|(_, _, _, _, _, entry)| entry)
        .collect()
}
