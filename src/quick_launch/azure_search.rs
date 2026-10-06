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
    let title_query = crate::azure_devops::TitleQuery::new(query);
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
            let title_quality = title_query.quality(&entry.name, Some(fields.name));
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

/// `az ` 直後の Recent 区分に出す、種別ごとの上限件数 (FR-9.18.7)。
const RECENT_PER_KIND: usize = 3;

impl super::Index {
    /// `az ` 直後 (検索語なし) の区分見出し付き一覧 (FR-9.18.7)。
    ///
    /// 使用履歴のある PR / Work Item を種別ごとに上位 `RECENT_PER_KIND` 件
    /// 「Recent」へ、続けて固定のショートカット・コマンド候補を「Commands」へ
    /// 並べる。出す Recent が無い (設定オフ・初回) ときは `None` を返し、
    /// 呼び出し側は従来の見出しなし一覧へ落とす。履歴は索引構築時に読み込み済みの
    /// `Ranking` を引くだけなので、ここで I/O はしない。
    pub fn azure_sections(&self) -> Option<Vec<(&'static str, Vec<&Entry>)>> {
        if !self.azure_recent {
            return None;
        }
        let pull_requests = self
            .azure
            .iter()
            .filter(|item| item.kind == crate::azure_devops::Kind::PullRequest)
            .map(|item| (&item.entry, &item.lower));
        let work_items = self
            .azure_work_items
            .iter()
            .zip(&self.azure_work_items_lower);
        let mut recent = self.recent_top(pull_requests);
        recent.extend(self.recent_top(work_items));
        if recent.is_empty() {
            return None;
        }
        let commands = self
            .azure_shortcuts
            .iter()
            .chain(super::azure_entries::azure_command_entries())
            .collect();
        Some(vec![("Recent", recent), ("Commands", commands)])
    }

    /// 履歴のある候補だけを (使用回数, 最終選択) の順位で並べ、上位を返す。
    fn recent_top<'a>(
        &self,
        items: impl Iterator<Item = (&'a Entry, &'a LowerKeys)>,
    ) -> Vec<&'a Entry> {
        let mut used = items
            .filter_map(|(entry, keys)| {
                let rank = self.ranking.rank_lower(entry, &keys.path);
                (rank != (u64::MAX, u64::MAX)).then_some((rank, entry))
            })
            .collect::<Vec<_>>();
        used.sort_by_key(|(rank, _)| *rank);
        used.into_iter()
            .take(RECENT_PER_KIND)
            .map(|(_, entry)| entry)
            .collect()
    }
}
