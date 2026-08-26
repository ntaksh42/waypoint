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
use super::bench_fixture::{large_index, time};
use std::time::Instant;

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

/// 実機の設定・ブックマーク・履歴・アプリをそのまま読んで検索する。
/// 合成データのベンチが実運用と乖離していないかの答え合わせ。
///
/// 実測 (entries 22 / bookmarks 18 / history 1000 / apps 87 / windows 6):
/// 1 打鍵あたり 0.014〜0.032ms。表示予算 50ms の 0.03% 程度でしかない。
///
/// **合成データのベンチ (候補 5500 件) は実機の 20 倍以上の規模がある。**
/// あちらで見えた 1〜2ms 級の差は、この規模では 0.02ms 級に縮む。検索の
/// 最適化はヘッドルーム確保としては意味があるが、実運用のボトルネックは
/// 検索ではない。効くのは `Index::build` を UI スレッドで何度も呼ばない
/// こと (91ms / 35ms 対 軽量版 0.001ms) の方。判断を誤らないよう、
/// 合成ベンチの数字を見るときは必ずこちらと並べること。
#[test]
#[ignore = "手動計測用"]
fn bench_real_config_search() {
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let config = match crate::config::load() {
        crate::config::LoadOutcome::Loaded(config)
        | crate::config::LoadOutcome::Created(config) => config,
        crate::config::LoadOutcome::Failed(error) => {
            println!("(実機の config を読めないので skip: {error})");
            return;
        }
    };
    let dynamic = crate::dynamic::refresh();
    let index = Index::build(&config, &dynamic);
    println!(
        "実機 index: entries={} bookmarks={} history={} apps={} windows={}",
        index.entries.len(),
        index.bookmarks.len(),
        index.history.len(),
        index.apps.len(),
        index.windows.len(),
    );

    for query in ["w", "wa", "way", "waypoint", "src", "co", "zzqqxx"] {
        for _ in 0..3 {
            std::hint::black_box(index.search(query));
        }
        let start = Instant::now();
        for _ in 0..500 {
            std::hint::black_box(index.search(std::hint::black_box(query)));
        }
        let hits = index.search(query).len();
        println!(
            "  query={query:<10} hits={hits:<5} {:>8.4} ms/iter",
            start.elapsed().as_secs_f64() * 1000.0 / 500.0
        );
    }
    // 空クエリ (ウィンドウを開いた直後の表示)
    let start = Instant::now();
    for _ in 0..500 {
        std::hint::black_box(index.sections());
    }
    println!(
        "  sections (空クエリ)         {:>8.4} ms/iter",
        start.elapsed().as_secs_f64() * 1000.0 / 500.0
    );
}

/// ホットキーを押してから一覧が出るまでに走る計算部分。
///
/// Win32 のウィンドウ生成・表示は測れないので、`show()` が同期で誘発する
/// 計算 (`SetWindowTextW("")` → `EN_CHANGE` → `update_results` →
/// `sections()`) を実機データで測る。表示予算 50ms に対する余裕を見る。
#[test]
#[ignore = "手動計測用"]
fn bench_show_path_compute() {
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let config = match crate::config::load() {
        crate::config::LoadOutcome::Loaded(config)
        | crate::config::LoadOutcome::Created(config) => config,
        crate::config::LoadOutcome::Failed(_) => return,
    };
    let dynamic = crate::dynamic::refresh();
    let index = Index::build(&config, &dynamic);

    // 表示直後の一覧 = 空クエリの sections
    for _ in 0..3 {
        std::hint::black_box(index.sections());
    }
    let start = Instant::now();
    for _ in 0..500 {
        std::hint::black_box(index.sections());
    }
    let sections = start.elapsed().as_secs_f64() * 1000.0 / 500.0;
    println!(
        "表示直後の一覧 (sections) {sections:>8.4} ms  — 表示予算 50ms に対し {:.2}%",
        sections / 50.0 * 100.0
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
