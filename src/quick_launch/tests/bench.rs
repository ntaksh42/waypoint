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
    // 逐次入力 (実際のキー入力を模す)
    let start = Instant::now();
    for n in 1..="projectfolder".len() {
        let q = &"projectfolder"[..n];
        std::hint::black_box(index.search(q));
    }
    println!(
        "typing 'projectfolder' (13 keys)  total {:>8.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
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
