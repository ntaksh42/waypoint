//! 検索経路の手動計測。`cargo test --release -- --ignored --nocapture bench_`
//! で走らせる。CI では走らない (時間依存なので合否判定はしない)。

use super::super::*;
use std::time::Instant;

/// 実運用規模 (ブックマーク数千・履歴数千・アプリ数百) を模した索引。
fn large_index(entries: usize, bookmarks: usize, history: usize, apps: usize) -> Index {
    fn folder(i: usize) -> Entry {
        Entry {
            name: format!("Project Folder {i}"),
            breadcrumb: format!("Projects > Group {}", i % 32),
            path: format!(r"E:\projects\group{}\project-{i}\src", i % 32),
            action: Action::OpenFolder(crate::config::OpenMode::NewWindow),
            branch: None,
        }
    }
    fn url(i: usize, kind: &str) -> Entry {
        let u = format!("https://example{}.com/{kind}/page/{i}?q=value", i % 64);
        Entry {
            name: format!("{kind} Bookmark Title Number {i}"),
            breadcrumb: format!("{kind} > Folder {}", i % 16),
            path: u.clone(),
            action: Action::OpenUrl(u),
            branch: None,
        }
    }
    fn app(i: usize) -> Entry {
        Entry {
            name: format!("Application Number {i}"),
            breadcrumb: String::new(),
            path: format!(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\App{i}.lnk"),
            action: Action::LaunchApp,
            branch: None,
        }
    }

    let mut index = Index {
        entries: (0..entries).map(folder).collect(),
        bookmarks: (0..bookmarks).map(|i| url(i, "Bookmarks")).collect(),
        history: (0..history).map(|i| url(i, "History")).collect(),
        apps: (0..apps).map(app).collect(),
        search_paths: true,
        ..Default::default()
    };
    index.entries_lower = super::super::search::LowerKeys::build_for(&index.entries);
    index.bookmarks_lower = super::super::search::LowerKeys::build_for(&index.bookmarks);
    index.history_lower = super::super::search::LowerKeys::build_for(&index.history);
    index.apps_lower = super::super::search::LowerKeys::build_for(&index.apps);
    index
}

fn time(label: &str, index: &Index, query: &str, iterations: u32) {
    // ウォームアップ (FUZZY_MATCHER の遅延初期化を計測から外す)
    let mut hits = 0;
    for _ in 0..3 {
        hits = index.search(query).len();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(index.search(std::hint::black_box(query)));
    }
    let elapsed = start.elapsed();
    println!(
        "{label:<34} query={query:<14} hits={hits:<6} {:>8.3} ms/iter",
        elapsed.as_secs_f64() * 1000.0 / f64::from(iterations)
    );
}

#[test]
#[ignore = "手動計測用"]
fn bench_search() {
    let index = large_index(2000, 3000, 5000, 500);
    println!(
        "index: entries={} bookmarks={} history={} apps={}",
        index.entries.len(),
        index.bookmarks.len(),
        index.history.len(),
        index.apps.len()
    );
    // 無接頭辞 (Folders + Windows + Bookmarks + Apps を横断)
    time("no-prefix 1 char", &index, "p", 200);
    time("no-prefix 3 chars", &index, "pro", 200);
    time("no-prefix typical", &index, "project", 200);
    time("no-prefix no-hit", &index, "zzqqxx", 200);
    time("no-prefix two terms", &index, "project 42", 200);
    // プレフィックス
    time("history prefix", &index, "h page", 200);
    time("bookmarks prefix", &index, "b bookmark", 200);
    time("apps prefix", &index, "a app", 200);
    // 各打鍵で毎回インデックス全体を引き直した場合のコスト。
    //
    // 実際の Quick Launch はこの経路を通らない。2 文字目以降は前回の結果
    // (最大 24 件) だけを絞り込む (`refined_search_term`)。実経路の計測は
    // `bench_incremental_typing` を見ること。ここは「絞り込みが効かない
    // 場合の上限」を見るための数値。
    let word = "projectfolder";
    let mut total = 0.0;
    let mut per_key = String::new();
    for n in 1..=word.len() {
        let q = &word[..n];
        for _ in 0..3 {
            std::hint::black_box(index.search(q));
        }
        let start = Instant::now();
        for _ in 0..50 {
            std::hint::black_box(index.search(std::hint::black_box(q)));
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
        total += ms;
        per_key.push_str(&format!("{ms:.2} "));
    }
    println!("typing '{word}' per-key ms: {per_key}");
    println!(
        "typing '{word}' ({} keys) total {total:>8.3} ms",
        word.len()
    );
}

#[test]
#[ignore = "手動計測用"]
fn bench_sections() {
    let index = large_index(2000, 3000, 5000, 500);
    let mut n = 0;
    for _ in 0..3 {
        n = index.sections().len();
    }
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(index.sections());
    }
    println!(
        "sections (empty query) sections={n} {:>8.3} ms/iter",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );
}

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

/// 実際のキー入力経路の模擬。`quick_launch_window::search` は 2 文字目以降、
/// 前回の結果だけを `search_entries` で絞り込む (`refined_search_term`)。
/// この絞り込み経路は `LowerKeys` を使わず毎回 `to_lowercase` する。
#[test]
#[ignore = "手動計測用"]
fn bench_incremental_typing() {
    use std::time::Instant;
    const MAX_LIST_RESULTS: usize = 24;
    let index = large_index(2000, 3000, 5000, 500);

    for word in ["projectfolder", "project", "application", "bookmark"] {
        // 実経路: 1 文字目は index.search、以降は前回結果を search_entries で絞る
        let start = Instant::now();
        let mut results: Vec<Entry> = Vec::new();
        let mut per_key = String::new();
        for n in 1..=word.len() {
            let query = &word[..n];
            let key_start = Instant::now();
            results = if n == 1 {
                index
                    .search(query)
                    .into_iter()
                    .take(MAX_LIST_RESULTS)
                    .cloned()
                    .collect()
            } else {
                crate::quick_launch::search_entries(
                    &results,
                    query,
                    index.search_paths,
                    &index.ranking,
                )
                .into_iter()
                .take(MAX_LIST_RESULTS)
                .cloned()
                .collect()
            };
            per_key.push_str(&format!(
                "{:.2} ",
                key_start.elapsed().as_secs_f64() * 1000.0
            ));
        }
        println!(
            "incremental '{word}' total {:>7.3} ms  per-key: {per_key}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// 索引構築のうち、動的な差し替え (`refresh_dynamic`) で毎回走る部分。
/// Recent/Frequent Folders と開いているウィンドウが変わるたびに走る。
#[test]
#[ignore = "手動計測用"]
fn bench_index_rebuild() {
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);

    let entries = index.entries.clone();
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(super::super::search::LowerKeys::build_for(&entries));
    }
    println!(
        "LowerKeys::build_for({})  {:>7.3} ms",
        entries.len(),
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );

    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(super::super::search::bench_dedup(entries.clone()));
    }
    let dedup = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(entries.clone());
    }
    let clone_only = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
    println!(
        "dedup_by_path({})         {:>7.3} ms  (clone {:.3} ms)",
        entries.len(),
        dedup - clone_only,
        clone_only
    );

    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(Ranking::default());
    }
    println!(
        "Ranking::default()        {:>7.3} ms",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );
}
