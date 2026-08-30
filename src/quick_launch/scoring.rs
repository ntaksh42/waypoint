//! 文字列一致の質を決めるスコアリング。
//!
//! 検索対象の集合や順位付けには関わらず、「候補 1 件の 1 語」に対して
//! どのティアで一致するかだけを見る純関数群。`search.rs` から切り出した。
//!
//! ティアはソートキーの第一要素になる (`search::rank_matches`)。小さいほど
//! 上位で、tier0 完全一致 〜 tier5 パス部分一致が安価な判定、tier6〜8 が
//! fuzzy (Skim の DP)、tier9 がローマ字。fuzzy は安価なティアより必ず
//! 下位に並ぶため、上位で表示ぶんが埋まれば評価を省ける
//! (`search::FUZZY_SKIP_THRESHOLD`)。

use fuzzy_matcher::FuzzyMatcher;

use super::FUZZY_MATCHER;

pub(super) fn match_score(
    name: &str,
    breadcrumb: &str,
    path: Option<&str>,
    term: &str,
) -> Option<(u8, i64)> {
    match_score_cheap(name, breadcrumb, path, term)
        .or_else(|| match_score_fuzzy(name, breadcrumb, path, term))
}

/// tier0〜5 (完全一致・前方一致・境界一致・部分一致・breadcrumb・path) と
/// tier9 (ローマ字) だけを見る、DP を伴わない安価な判定。
///
/// tier6〜8 の fuzzy より上位のティアはここで全て決まる。fuzzy を飛ばして
/// よいか判定するため (`fuzzy_needed`)、fuzzy 抜きで単独に呼べるよう
/// 分離してある。
pub(super) fn match_score_cheap(
    name: &str,
    breadcrumb: &str,
    path: Option<&str>,
    term: &str,
) -> Option<(u8, i64)> {
    if name == term {
        Some((0, 0))
    } else if name.starts_with(term) {
        Some((1, 0))
    // tier2 (境界一致) と tier3 (部分一致) は両方とも `name` 中の出現位置を
    // 見るだけなので、`match_indices` を 1 回だけ回して同時に判定する
    // (以前は `match_indices` → `contains` の 2 回スキャンだった)。
    } else if let Some(tier) = name_contains_tier(name, term) {
        Some((tier, 0))
    } else if breadcrumb.contains(term) {
        Some((4, 0))
    } else if path.is_some_and(|path| path.contains(term)) {
        Some((5, 0))
    } else {
        None
    }
}

/// tier6〜8 (fuzzy) と tier9 (ローマ字)。`match_score_cheap` が `None` を
/// 返した候補にだけ意味がある。
///
/// fuzzy_match は候補文字列ぶんの `Vec<char>` 確保を伴う DP なので、term の
/// 各文字が順序を保って含まれているか (サブシーケンス) を先に O(n) で
/// 見て、成立しない候補は呼び出しごと避ける。fuzzy_matcher 内部の
/// cheap_matches も同じ判定をするが、そちらは choice を `Vec<char>` 化して
/// からでないと判定できない。name/breadcrumb/path は候補・語ともに
/// 既に小文字化済みなので大小無視のケース分けは不要。
fn match_score_fuzzy(
    name: &str,
    breadcrumb: &str,
    path: Option<&str>,
    term: &str,
) -> Option<(u8, i64)> {
    if is_subsequence(name, term) {
        FUZZY_MATCHER
            .fuzzy_match(name, term)
            .map(|score| (6, score))
    } else if is_subsequence(breadcrumb, term) {
        FUZZY_MATCHER
            .fuzzy_match(breadcrumb, term)
            .map(|score| (7, score))
    } else if path.is_some_and(|path| is_subsequence(path, term)) {
        path.and_then(|path| FUZZY_MATCHER.fuzzy_match(path, term))
            .map(|score| (8, score))
    } else if crate::romaji::kana_name_matches(name, term) {
        Some((9, 0))
    } else {
        None
    }
}

/// `term` の全文字が `text` 中に順序を保って (連続していなくてよい) 現れるか。
/// fuzzy_match を試す価値があるかどうかの安価な事前判定。
///
/// 候補・語ともに小文字化済みで、実際の入力はほぼ ASCII のため、両方が
/// ASCII の場合はバイト列として走査する。`chars()` の UTF-8 デコードを挟むと
/// 1 バイトずつ分岐が入って自動ベクトル化が効かず、ここが無一致クエリの
/// コストの大半を占めていた (実測: 5500 候補 x 3 フィールドで 1.375ms、
/// `match_score` 全体 1.73ms の約 8 割)。
fn is_subsequence(text: &str, term: &str) -> bool {
    if text.is_ascii() && term.is_ascii() {
        return is_subsequence_ascii(text.as_bytes(), term.as_bytes());
    }
    let mut chars = text.chars();
    term.chars().all(|t| chars.any(|c| c == t))
}

/// ASCII 限定のサブシーケンス判定。語の各バイトを `memchr` 相当の
/// バイト検索 (`iter().position`) で順に追う。
fn is_subsequence_ascii(text: &[u8], term: &[u8]) -> bool {
    let mut rest = text;
    for &want in term {
        match rest.iter().position(|&c| c == want) {
            Some(at) => rest = &rest[at + 1..],
            None => return false,
        }
    }
    true
}

/// `name` が `term` を含むときそのティアを返す (境界一致なら tier2、
/// それ以外の部分一致なら tier3)。含まなければ `None`。
/// 出現位置の走査を 1 回にまとめ、tier2/tier3 で `name` を二度スキャンしない。
fn name_contains_tier(name: &str, term: &str) -> Option<u8> {
    let mut found = false;
    for (index, _) in name.match_indices(term) {
        if index == 0 || name[..index].ends_with([' ', '-', '_', '.']) {
            return Some(2);
        }
        found = true;
    }
    found.then_some(3)
}

/// `name` 中で検索語がヒットしたバイト範囲 (候補一覧のハイライト表示用)。
///
/// `term` はモードプレフィックス (`b `/`az pr ` 等) を除いた実際の検索語
/// (`super::effective_search_term`)。複数語クエリは語ごとの範囲を合算する。
/// breadcrumb/path/ローマ字経由の一致 (tier4/5/7/8/9) は `name` 中に対応する
/// 位置が無いため対象外 (ハイライトなし)。
pub(crate) fn highlight_ranges(name: &str, term: &str) -> Vec<(usize, usize)> {
    if term.trim().is_empty() {
        return Vec::new();
    }
    let name_lower = name.to_lowercase();
    let mut ranges = Vec::new();
    for word in term.split_whitespace().map(str::to_lowercase) {
        ranges.extend(term_ranges(name, &name_lower, &word));
    }
    merge_ranges(ranges)
}

/// 1 語ぶんのヒット範囲。連続部分一致 (tier0〜3 相当) を優先し、
/// 無ければ fuzzy のサブシーケンス一致 (tier6 相当) を試す。
fn term_ranges(name: &str, name_lower: &str, word: &str) -> Vec<(usize, usize)> {
    if let Some(start) = name_lower.find(word) {
        return vec![(start, start + word.len())];
    }
    let Some((_, char_indices)) = FUZZY_MATCHER.fuzzy_indices(name_lower, word) else {
        return Vec::new();
    };
    char_indices_to_byte_ranges(name, &char_indices)
}

/// fuzzy_matcher が返す文字インデックス (昇順) を、連続する run 単位で
/// `name` のバイト範囲へ変換する。飛び飛びのマッチをセグメントごとに
/// 分けることで、VSCode 同様「実際に一致した文字だけ」を強調できる。
fn char_indices_to_byte_ranges(name: &str, char_indices: &[usize]) -> Vec<(usize, usize)> {
    let mut byte_offsets: Vec<usize> = name.char_indices().map(|(byte, _)| byte).collect();
    byte_offsets.push(name.len());
    let mut ranges = Vec::new();
    let mut iter = char_indices.iter().copied();
    let Some(mut run_start) = iter.next() else {
        return ranges;
    };
    let mut run_end = run_start + 1;
    for idx in iter {
        if idx == run_end {
            run_end = idx + 1;
        } else {
            push_byte_range(&mut ranges, &byte_offsets, run_start, run_end);
            run_start = idx;
            run_end = idx + 1;
        }
    }
    push_byte_range(&mut ranges, &byte_offsets, run_start, run_end);
    ranges
}

/// `to_lowercase` で文字数が変わる極端なケース (独語 `ß` → `ss` 等) に
/// 備え、`byte_offsets` の範囲外になる添字は捨てる。
fn push_byte_range(
    ranges: &mut Vec<(usize, usize)>,
    byte_offsets: &[usize],
    start: usize,
    end: usize,
) {
    if let (Some(&start), Some(&end)) = (byte_offsets.get(start), byte_offsets.get(end)) {
        ranges.push((start, end));
    }
}

/// 範囲を開始位置でソートし、隣接・重複する範囲を 1 つへ統合する。
fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    merged
}

#[cfg(test)]
pub(crate) fn bench_is_subsequence(text: &str, term: &str) -> bool {
    is_subsequence(text, term)
}

#[cfg(test)]
pub(crate) fn bench_match_score(
    name: &str,
    breadcrumb: &str,
    path: Option<&str>,
    term: &str,
) -> Option<(u8, i64)> {
    match_score(name, breadcrumb, path, term)
}
