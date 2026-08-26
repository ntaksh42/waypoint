//! 検索経路の手動計測。`cargo test --release -- --ignored --nocapture bench_`
//! で走らせる。CI では走らない (時間依存なので合否判定はしない)。
//!
//! 基準値 (候補 5500 件: entries 2000 / bookmarks 3000 / apps 500)。
//! 0bf384f (最適化前) と 17a3b51 を同一機・同一ベンチで測った実測値:
//!
//! | 計測 | 0bf384f | 17a3b51 |
//! |---|---|---|
//! | search "pro" | 1.164 ms | 0.185 ms |
//! | search "project" | 1.339 ms | 0.322 ms |
//! | search "zzqqxx" (無一致) | 1.776 ms | 0.999 ms |
//! | search "project 42" | 1.826 ms | 1.035 ms |
//! | 13 打鍵 (毎回引き直し) | 35.1 ms | 14.5 ms |
//! | "application" (実経路) | 0.928 ms | 0.517 ms |
//!
//! 効いた変更は 3 つ: サブシーケンス判定の ASCII バイト走査化、
//! 上位ティアで埋まったときの fuzzy 打ち切り、`opt-level` を 3 へ。

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

/// 空クエリ (`sections`) の内訳。全語一致の判定は走らないが、
/// 候補全件に `rank_lower` をかけて全件ソートしている。
#[test]
#[ignore = "手動計測用"]
fn bench_sections_breakdown() {
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);
    let total: usize =
        index.entries.len() + index.bookmarks.len() + index.history.len() + index.apps.len();
    println!("sections が触る候補の総数 = {total}");

    // rank_lower 全件
    let start = Instant::now();
    for _ in 0..100 {
        let mut acc = 0u64;
        for (e, k) in index.entries.iter().zip(&index.entries_lower) {
            acc = acc.wrapping_add(
                index
                    .ranking
                    .rank_lower(e, super::super::search::keys_path(k))
                    .0,
            );
        }
        std::hint::black_box(acc);
    }
    println!(
        "rank_lower x{}  {:>7.3} ms",
        index.entries.len(),
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );

    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(index.sections());
    }
    println!(
        "sections()      {:>7.3} ms",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );
}

/// スタートメニューの `.lnk` 列挙 (`a ` プレフィックスの母集団)。
/// `Index::build` で走る。実機のスタートメニューをそのまま読む。
///
/// コストのほぼ全部 (実測 30ms 中 27ms) が `.lnk` の実体解決で、
/// 内訳は `IPersistFile::Load` のファイル I/O。COM オブジェクトを
/// 1 個に使い回す・受け取りバッファを再利用するといった手は試したが
/// 速くならなかった (実測で差が出ず、複雑さだけ増えたので入れていない)。
/// `SLGP_RAWPATH` は環境変数が展開されず、正しいアプリまで壊れた
/// リンク扱いで落ちるので使えない (実測: 87 件 → 45 件)。
///
/// したがってここは「速くする」より「UI スレッドで何度も走らせない」
/// 方向で抑える (`Index::refresh_azure` / `refresh_dynamic` 参照)。
#[test]
#[ignore = "手動計測用"]
fn bench_apps_scan() {
    use std::time::Instant;
    // COM を使うので初期化しておく (常駐部は UI スレッドで初期化済み)。
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let start = Instant::now();
    let apps = crate::apps::scan();
    let first = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let again = crate::apps::scan();
    let second = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "apps::scan() {} 件  1 回目 {first:>8.2} ms  2 回目 {second:>8.2} ms",
        apps.len()
    );
    assert_eq!(apps.len(), again.len());
}

/// フル `Index::build` の実測。実機のブックマーク・履歴・スタートメニューを
/// 読むので、起動時と設定リロード時のコストがそのまま出る。
///
/// この経路をユーザー操作のたびに通さないことが重要で、そのために
/// `refresh_dynamic` / `refresh_azure` / `refresh_config_items` がある。
#[test]
#[ignore = "手動計測用"]
fn bench_index_build_full() {
    use crate::config::Config;
    use crate::dynamic::Menus;
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let config = Config::default();
    let dynamic = Menus::default();

    let start = Instant::now();
    let index = Index::build(&config, &dynamic);
    let cold = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let warm_index = Index::build(&config, &dynamic);
    let warm = start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "Index::build 1 回目 {cold:>8.2} ms  2 回目 {warm:>8.2} ms  (entries={} bookmarks={} history={} apps={})",
        index.entries.len(),
        index.bookmarks.len(),
        index.history.len(),
        index.apps.len(),
    );

    // 軽量版との比較
    let mut index = warm_index;
    let start = Instant::now();
    index.refresh_dynamic(&config, &dynamic);
    let dynamic_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    index.refresh_config_items(&config, &dynamic);
    let config_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("refresh_dynamic {dynamic_ms:>8.3} ms  refresh_config_items {config_ms:>8.3} ms");
}

/// 1 行描くたびに走る `Entry` の複製。`draw_list_item` は借用を解放してから
/// 描くために結果を clone しており、再描画のたびに行数ぶん走る
/// (`LBS_OWNERDRAWVARIABLE` なので打鍵ごとの `LB_RESETCONTENT` で全行再描画)。
///
/// 結論: 再描画 1 回ぶんで 0.0016ms しかかからないので手を入れない。
/// この clone は再入 (`RefCell` の借用を解放してから描く) のために
/// あるので、消すと `quick_launch_reentrancy_test` が守っている性質を
/// 壊す。コストが無視できる以上、触る理由がない。
#[test]
#[ignore = "手動計測用"]
fn bench_row_entry_clone() {
    use std::time::Instant;
    const VISIBLE_ROWS: usize = 24;
    let index = large_index(2000, 3000, 5000, 500);
    let results: Vec<Entry> = index
        .search("project")
        .into_iter()
        .take(VISIBLE_ROWS)
        .cloned()
        .collect();

    let start = Instant::now();
    for _ in 0..2000 {
        // 1 回の再描画ぶん (可視行すべて)
        for entry in &results {
            std::hint::black_box(entry.clone());
        }
    }
    println!(
        "Entry::clone x{VISIBLE_ROWS} (再描画 1 回ぶん) {:>8.4} ms",
        start.elapsed().as_secs_f64() * 1000.0 / 2000.0
    );
}
