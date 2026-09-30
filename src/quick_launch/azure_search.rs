//! Azure DevOps 候補専用の検索順位付け。

use std::cmp::Reverse;

use super::Entry;
use super::rank::{Fuzzy, lower_terms, score_entry};
use super::search::{Fields, LowerKeys};
use crate::quick_launch_history::Ranking;

const AZURE_URL_PREFIX: &str = "https://dev.azure.com/";

/// 語句一致と全語タイトル一致を breadcrumb / URL の一致より優先し、
/// 1 文字のタイプミスも拾う。一致の質が同じなら、自分が関わる Active PR →
/// その他の未完了 → 完了済みの順に並べる (`AzureMeta::state_rank`)。
/// breadcrumb / URL は部分文字列一致だけで拾い、fuzzy は名前にだけ当てる。
pub(super) fn search<'a>(
    items: impl IntoIterator<Item = (&'a Entry, &'a LowerKeys)>,
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms = lower_terms(query);
    // `#<番号>` は番号の完全一致だけを返す。breadcrumb / URL 側の一致で残さない。
    let exact_id = crate::azure_devops::exact_id_query(query).is_some();
    let mut matches = items
        .into_iter()
        .enumerate()
        .filter_map(|(order, (entry, keys))| {
            let mut fields = Fields::from_keys(keys, search_paths);
            // 全候補に共通の `https://dev.azure.com/` は照合から外す (`az azure` で
            // 全件が一致しないように)。`path_lower` は使用履歴のキーなので触らない。
            fields.path = fields
                .path
                .map(|path| path.strip_prefix(AZURE_URL_PREFIX).unwrap_or(path));
            let title_quality = crate::azure_devops::title_match_quality(&entry.name, query);
            let general = (!exact_id)
                .then(|| score_entry(entry, fields, &terms, ranking, Fuzzy::NameOnly))
                .flatten();
            let (tier, fuzzy_score, usage) = general
                .unwrap_or_else(|| (u8::MAX, 0, ranking.rank_lower(entry, fields.path_lower)));
            let state_rank = entry.azure.as_ref().map_or(1, |meta| meta.state_rank());
            title_quality
                .or_else(|| general.map(|_| crate::azure_devops::TITLE_QUALITY_OTHER))
                .map(|title_quality| {
                    (
                        (
                            title_quality,
                            state_rank,
                            tier,
                            Reverse(fuzzy_score),
                            usage,
                            order,
                        ),
                        entry,
                    )
                })
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(key, _)| *key);
    matches.into_iter().map(|(_, entry)| entry).collect()
}
