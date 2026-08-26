//! ベンチ共通の道具立て。実運用規模を模した索引と計測ヘルパー。
//!
//! 計測本体は用途ごとに分かれている:
//! `bench_search` (検索の速さ) / `bench_scoring` (検索コストの内訳) /
//! `bench_io` (索引構築・ファイル/COM I/O・描画)。

use super::super::*;
use std::time::Instant;

/// 実運用規模 (ブックマーク数千・履歴数千・アプリ数百) を模した索引。
pub(super) fn large_index(entries: usize, bookmarks: usize, history: usize, apps: usize) -> Index {
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

pub(super) fn time(label: &str, index: &Index, query: &str, iterations: u32) {
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
