//! 候補のスコアリングと並べ替え。
//!
//! `search.rs` が 500 行規約を超えたため分離した。`Index` の検索経路
//! (`search.rs`) から呼ばれる、候補集合に対する採点・絞り込み・整列の
//! 実装だけを置く。

use std::cmp::Reverse;

use super::Entry;
use super::scoring::{match_score, match_score_cheap};
use super::search::{Fields, LowerKeys};
use crate::quick_launch_history::Ranking;

/// スコアリングで fuzzy (tier6〜8) を評価するかどうか。
///
/// fuzzy は Skim の DP で、候補文字列ぶんの `Vec<char>` 確保を伴う。実測で
/// 候補 2000 件に対し 2.3ms と、安価なティア判定 (0.1ms) の 20 倍以上かかる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fuzzy {
    Include,
    Skip,
}

/// 安価なティア (0〜5) の一致がこの件数に達したら、fuzzy を評価しない。
///
/// ソートキーの第一要素は tier で、fuzzy は tier6〜8 と必ず tier0〜5 より
/// 下位に並ぶ。表示側 (`quick_launch_window::MAX_LIST_RESULTS` = 24) が
/// 使うのは上位 24 件だけなので、tier5 以下で 24 件揃っていれば fuzzy の
/// 結果は一覧に載り得ず、計算しても捨てるだけになる。
///
/// 表示上限に直結させず余裕を持たせてあるのは、表示側を増やしたときに
/// 静かに候補が減らないようにするため。
const FUZZY_SKIP_THRESHOLD: usize = 64;

/// 候補 1 件のスコア (検索一致の質、Fuzzy スコア、使用履歴)。
pub(super) type Score = (u8, i64, (u64, u64));

/// スコア付きの一致。`rank_matches` へ渡す前の中間表現。
type ScoredMatch<'a> = (Score, usize, &'a Entry);

/// 検索一致の質、Fuzzy スコア、使用履歴、元の順序と候補本体。
type SearchMatch<'a> = (u8, i64, (u64, u64), usize, &'a Entry);

pub(crate) fn search_entries<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms = lower_terms(query);
    rank_matches(
        entries
            .into_iter()
            .enumerate()
            .filter_map(|(order, entry)| {
                // ranking.rank も path の小文字化を要求するため、search_paths に
                // 関わらず 1 回だけ計算して使い回す (rank 側で to_lowercase を
                // やり直すと、事前計算キャッシュを通らないこの経路のコストが
                // さらに増えてしまう)。
                let keys = LowerKeys::new(entry);
                score_entry(
                    entry,
                    Fields::from_keys(&keys, search_paths),
                    &terms,
                    ranking,
                    Fuzzy::Include,
                )
                .map(|score| (score, order, entry))
            }),
    )
}

/// `LowerKeys` で事前計算済みの候補群を検索する。`entries` と `lower_keys`
/// は同じ順序・同じ長さである前提 (`Index::build` が対で作る)。長さが
/// 合わない場合 (テストで `Index` を直接組み立て、キャッシュだけ未構築で
/// 残した場合など) は安全側に倒し、都度計算の `search_entries` へ回す。
pub(crate) fn search_entries_cached<'a>(
    entries: &'a [Entry],
    lower_keys: &[LowerKeys],
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    search_entries_cached_multi(&[(entries, lower_keys)], query, search_paths, ranking)
}

/// 複数の候補群 (`entries` と対応する `lower_keys` の組) を横断して検索する。
/// 無接頭辞の通常検索が Folders / Open Windows / Bookmarks / Apps を
/// 一括で検索する経路向け。組ごとに `entries.len() != lower_keys.len()`
/// なら都度計算にフォールバックする (`search_entries_cached` と同じ規約)。
/// `order` はソースをまたいで通し番号にし、ソース内の元の並びと
/// ソースの列挙順を同点時の並び順として保つ。
pub(crate) fn search_entries_cached_multi<'a>(
    sources: &[(&'a [Entry], &[LowerKeys])],
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms = lower_terms(query);
    // fuzzy (tier6〜8) は Skim の DP で、候補文字列ぶんの `Vec<char>` 確保を
    // 伴う。実測で候補 2000 件に対し 2.3ms と、安価なティア (0〜5) の判定
    // (0.4ms) の 5 倍以上かかる。
    //
    // 一方でソートキーの第一要素は tier なので、tier5 以下で表示ぶん
    // (`FUZZY_SKIP_THRESHOLD`) が埋まれば fuzzy の結果は一覧に載り得ない。
    // そこで 1 巡目は fuzzy を飛ばし、埋まったらそのまま返す。埋まらなければ
    // fuzzy 込みで組み直す。
    //
    // 組み直しでは tier0〜5 の判定をやり直すことになる。外れた候補だけを
    // 控えて 2 巡目へ回す形も試したが、控えるための `Vec` のコストが
    // 再判定の節約を上回り速くならなかった (実測: `zzqqxx` でどちらも
    // 約 1.18ms)。単純な方を採る。
    let mut scored = scan_sources(sources, &terms, search_paths, ranking, Fuzzy::Skip);
    if scored.len() < FUZZY_SKIP_THRESHOLD {
        scored = scan_sources(sources, &terms, search_paths, ranking, Fuzzy::Include);
    }
    rank_matches(scored.into_iter())
}

/// `sources` を順に走査してスコア付きの一致を集める。`order` はソースを
/// またいで通し番号にし、ソース内の元の並びとソースの列挙順を同点時の
/// 並び順として保つ。
fn scan_sources<'a>(
    sources: &[(&'a [Entry], &[LowerKeys])],
    terms: &[String],
    search_paths: bool,
    ranking: &Ranking,
    fuzzy: Fuzzy,
) -> Vec<ScoredMatch<'a>> {
    let mut scored = Vec::new();
    let mut order = 0usize;
    for &(entries, lower_keys) in sources {
        // `entries.len() != lower_keys.len()` のときは都度計算にフォールバック
        // する (`search_entries_cached` と同じ規約)。
        let cached = entries.len() == lower_keys.len();
        for (position, entry) in entries.iter().enumerate() {
            let owned;
            let keys = if cached {
                &lower_keys[position]
            } else {
                owned = LowerKeys::new(entry);
                &owned
            };
            if let Some(score) = score_entry(
                entry,
                Fields::from_keys(keys, search_paths),
                terms,
                ranking,
                fuzzy,
            ) {
                scored.push((score, order, entry));
            }
            order += 1;
        }
    }
    scored
}

pub(super) fn lower_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// 全語をスコアリングし、複数語のときは一番弱い一致を順位に使う。
///
/// `path_lower` はマッチ対象 (`path`、`search_paths` が false なら None) とは
/// 別に常に渡す。使用履歴の順位付け (`ranking.rank_lower`) がパスの小文字化を
/// 要求するため、`path` が None のときも呼び出し側の事前計算済み値を使い回し、
/// ここで `entry.path.to_lowercase()` を再計算しないようにする。
pub(super) fn score_entry(
    entry: &Entry,
    fields: Fields<'_>,
    terms: &[String],
    ranking: &Ranking,
    fuzzy: Fuzzy,
) -> Option<Score> {
    let Fields {
        name,
        breadcrumb,
        path,
        path_lower,
    } = fields;
    // 中間 Vec を作らず、全語一致を要求しつつ一番弱い一致へ畳み込む
    // (`Option<Vec<_>>` の collect は語ごとに match_score を呼ぶのは同じだが、
    // クエリのたびに毎候補でヒープ確保が走っていた。単語 1 個の検索が
    // 最頻出のため、そのケースの割り当てを消す効果が大きい)。
    let mut result: Option<(u8, i64)> = None;
    for term in terms {
        let score = match fuzzy {
            Fuzzy::Include => match_score(name, breadcrumb, path, term)?,
            Fuzzy::Skip => match_score_cheap(name, breadcrumb, path, term)?,
        };
        result = Some(match result {
            Some(acc) => std::cmp::max_by_key(acc, score, |(tier, fuzzy_score)| {
                (*tier, Reverse(*fuzzy_score))
            }),
            None => score,
        });
    }
    let (tier, fuzzy_score) = result.unwrap_or((0, 0));
    Some((tier, fuzzy_score, ranking.rank_lower(entry, path_lower)))
}

/// 文字列一致の質を最優先し、同点内では使用頻度・最近使った順で並べる。
fn rank_matches<'a>(
    scored: impl Iterator<Item = ((u8, i64, (u64, u64)), usize, &'a Entry)>,
) -> Vec<&'a Entry> {
    let mut matches: Vec<SearchMatch<'a>> = scored
        .map(|((tier, fuzzy_score, usage), order, entry)| (tier, fuzzy_score, usage, order, entry))
        .collect();
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

#[cfg(test)]
pub(crate) fn bench_score_cheap(
    entry: &Entry,
    keys: &LowerKeys,
    search_paths: bool,
    terms: &[String],
    ranking: &Ranking,
) -> Option<(u8, i64, (u64, u64))> {
    score_entry(
        entry,
        Fields::from_keys(keys, search_paths),
        terms,
        ranking,
        Fuzzy::Skip,
    )
}

#[cfg(test)]
pub(crate) fn bench_cached_multi<'a>(
    sources: &[(&'a [Entry], &[LowerKeys])],
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    search_entries_cached_multi(sources, query, search_paths, ranking)
}

#[cfg(test)]
pub(crate) fn fuzzy_skip_threshold() -> usize {
    FUZZY_SKIP_THRESHOLD
}

#[cfg(test)]
pub(crate) fn bench_dedup(entries: Vec<Entry>) -> Vec<Entry> {
    dedup_by_path(entries)
}

#[cfg(test)]
pub(crate) fn keys_path(keys: &LowerKeys) -> &str {
    &keys.path
}
