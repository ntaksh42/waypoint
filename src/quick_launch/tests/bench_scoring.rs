//! 検索コストの内訳を切り分ける診断用ベンチ。
//!
//! どのティア・どの段が支配的かを見るためのもの。代表値は
//! `bench_search` を見ること。

use super::super::*;
use super::bench_fixture::large_index;

/// `match_score` の段階別コスト内訳。どのティアで時間を使っているか切り分ける。
#[test]
#[ignore = "手動計測用"]
fn bench_stages() {
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);
    let all: Vec<&Entry> = index
        .entries
        .iter()
        .chain(&index.bookmarks)
        .chain(&index.apps)
        .collect();
    let lower: Vec<(String, String, String)> = all
        .iter()
        .map(|e| {
            (
                e.name.to_lowercase(),
                e.breadcrumb.to_lowercase(),
                e.path.to_lowercase(),
            )
        })
        .collect();
    println!("candidates = {}", all.len());

    let term = "zzqqxx";
    macro_rules! stage {
        ($label:expr, $body:expr) => {{
            for _ in 0..3 {
                std::hint::black_box($body);
            }
            let start = Instant::now();
            for _ in 0..200 {
                std::hint::black_box($body);
            }
            println!(
                "{:<28} {:>8.3} ms/iter",
                $label,
                start.elapsed().as_secs_f64() * 1000.0 / 200.0
            );
        }};
    }

    stage!("contains name/bread/path", {
        let mut n = 0;
        for (name, bread, path) in &lower {
            if name.contains(term) || bread.contains(term) || path.contains(term) {
                n += 1;
            }
        }
        n
    });
    stage!("is_subsequence x3", {
        let mut n = 0;
        for (name, bread, path) in &lower {
            if super::super::search::bench_is_subsequence(name, term)
                || super::super::search::bench_is_subsequence(bread, term)
                || super::super::search::bench_is_subsequence(path, term)
            {
                n += 1;
            }
        }
        n
    });
    stage!("kana_name_matches", {
        let mut n = 0;
        for (name, _, _) in &lower {
            if crate::romaji::kana_name_matches(name, term) {
                n += 1;
            }
        }
        n
    });
    stage!("full match_score", {
        let mut n = 0;
        for (name, bread, path) in &lower {
            if super::super::search::bench_match_score(name, bread, Some(path.as_str()), term)
                .is_some()
            {
                n += 1;
            }
        }
        n
    });
    stage!("lower_terms alloc", {
        term.split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    });
    // 一致する語で fuzzy 経路を通す
    let hit = "prjct";
    stage!("full match_score (fuzzy hit)", {
        let mut n = 0;
        for (name, bread, path) in &lower {
            if super::super::search::bench_match_score(name, bread, Some(path.as_str()), hit)
                .is_some()
            {
                n += 1;
            }
        }
        n
    });
}

/// 全件ソート (`rank_matches`) が結果件数に対してどれだけ効いているか。
/// 表示は上位 24 件だけなので、ここが支配的なら部分ソートで削れる。
#[test]
#[ignore = "手動計測用"]
fn bench_sort_share() {
    use std::cmp::Reverse;
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);
    // 「p」は 5500 件全部が一致する = ソート対象が最大になるクエリ
    for query in ["p", "pro", "project"] {
        let hits = index.search(query).len();
        // 同じ規模のキーでソートだけを計測する
        let mut keys: Vec<(u8, i64, (u64, u64), usize)> = (0..hits)
            .map(|i| {
                (
                    (i % 9) as u8,
                    (i as i64 * 7919) % 1000,
                    (u64::MAX - (i as u64 % 50), u64::MAX),
                    i,
                )
            })
            .collect();
        let start = Instant::now();
        for _ in 0..200 {
            let mut v = keys.clone();
            v.sort_by_key(|(t, f, u, o)| (*t, Reverse(*f), *u, *o));
            std::hint::black_box(v);
        }
        let full = start.elapsed().as_secs_f64() * 1000.0 / 200.0;
        let start = Instant::now();
        for _ in 0..200 {
            let mut v = keys.clone();
            let n = v.len().min(24);
            if n < v.len() {
                v.select_nth_unstable_by_key(n, |(t, f, u, o)| (*t, Reverse(*f), *u, *o));
                v.truncate(n);
            }
            v.sort_by_key(|(t, f, u, o)| (*t, Reverse(*f), *u, *o));
            std::hint::black_box(v);
        }
        let partial = start.elapsed().as_secs_f64() * 1000.0 / 200.0;
        // clone のコストを引く
        let start = Instant::now();
        for _ in 0..200 {
            std::hint::black_box(keys.clone());
        }
        let clone_only = start.elapsed().as_secs_f64() * 1000.0 / 200.0;
        keys.clear();
        println!(
            "query={query:<9} hits={hits:<6} full-sort {:>7.3} ms  partial {:>7.3} ms  (clone {:>6.3} ms)",
            full - clone_only,
            partial - clone_only,
            clone_only
        );
    }
}

/// fuzzy_match (Skim の DP) が全体のどれだけを占めるか。
/// サブシーケンスが成立する候補が多いクエリで支配的になる。
#[test]
#[ignore = "手動計測用"]
fn bench_fuzzy_share() {
    use fuzzy_matcher::FuzzyMatcher;
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);
    let lower: Vec<String> = index
        .entries
        .iter()
        .chain(&index.bookmarks)
        .chain(&index.apps)
        .map(|e| e.name.to_lowercase())
        .collect();

    for term in ["projectfolder", "pr", "prjctfldr", "project"] {
        let subseq = lower
            .iter()
            .filter(|n| super::super::search::bench_is_subsequence(n, term))
            .count();
        let contains = lower.iter().filter(|n| n.contains(term)).count();
        // サブシーケンス判定だけ
        let start = Instant::now();
        for _ in 0..50 {
            let mut n = 0;
            for name in &lower {
                if super::super::search::bench_is_subsequence(name, term) {
                    n += 1;
                }
            }
            std::hint::black_box(n);
        }
        let sub_ms = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
        // サブシーケンス成立分に fuzzy_match をかける
        let start = Instant::now();
        for _ in 0..50 {
            let mut n = 0i64;
            for name in &lower {
                if super::super::search::bench_is_subsequence(name, term)
                    && let Some(s) = super::super::FUZZY_MATCHER.fuzzy_match(name, term)
                {
                    n += s;
                }
            }
            std::hint::black_box(n);
        }
        let fuzzy_ms = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
        println!(
            "term={term:<14} contains={contains:<5} subseq={subseq:<5} subseq-only {sub_ms:>6.3} ms  +fuzzy {fuzzy_ms:>6.3} ms  (fuzzy {:>6.3} ms)",
            fuzzy_ms - sub_ms
        );
    }
}

/// 2 巡目に落ちるクエリで、1 巡目 (cheap) 自体がいくらかかっているか。
#[test]
#[ignore = "手動計測用"]
fn bench_cheap_pass_cost() {
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);
    let sources: &[(&[Entry], &[super::super::search::LowerKeys])] = &[
        (&index.entries, &index.entries_lower),
        (&index.windows, &index.windows_lower),
        (&index.bookmarks, &index.bookmarks_lower),
        (&index.apps, &index.apps_lower),
    ];
    for term in ["zzqqxx", "project"] {
        let terms = vec![term.to_string()];
        let start = Instant::now();
        for _ in 0..200 {
            let mut n = 0usize;
            for &(entries, lower) in sources {
                for (entry, keys) in entries.iter().zip(lower) {
                    if super::super::search::bench_score_cheap(
                        entry,
                        keys,
                        true,
                        &terms,
                        &index.ranking,
                    )
                    .is_some()
                    {
                        n += 1;
                    }
                }
            }
            std::hint::black_box(n);
        }
        println!(
            "cheap pass term={term:<9} {:>7.3} ms",
            start.elapsed().as_secs_f64() * 1000.0 / 200.0
        );
    }
}
