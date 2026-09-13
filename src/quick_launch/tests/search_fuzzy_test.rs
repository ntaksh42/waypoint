//! fuzzy 枝刈りが結果を変えないことの確認。

use super::super::*;

/// fuzzy を飛ばす 1 巡目の最適化 (`FUZZY_SKIP_THRESHOLD`) が、検索結果の
/// 並びを変えないことを確かめる。
///
/// tier5 以下で表示ぶんが埋まったときに fuzzy (tier6〜8) を評価しない、
/// という枝刈りが成立するのは「ソートキーの第一要素が tier で、fuzzy は
/// 必ず tier5 より下位」だから。この前提が崩れれば結果が変わるので、
/// 常に fuzzy を評価する参照実装と突き合わせる。
#[test]
fn fuzzy_skip_does_not_change_results() {
    use super::super::search::LowerKeys;
    use super::super::*;

    // 閾値 (64) を超える件数の候補を用意し、1 巡目で打ち切られる経路と
    // 打ち切られない経路の両方を通す。
    let entries: Vec<Entry> = (0..200)
        .map(|i| Entry {
            name: format!("Project Folder {i}"),
            breadcrumb: format!("Projects > Group {}", i % 8),
            path: format!(r"E:\projects\group{}\proj-{i}", i % 8),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        })
        .chain((0..40).map(|i| Entry {
            name: format!("Zebra Archive {i}"),
            breadcrumb: "Archive".into(),
            path: format!(r"D:\archive\zebra-{i}"),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        }))
        .collect();
    let lower = LowerKeys::build_for(&entries);
    let ranking = Ranking::default();

    // 1 巡目で埋まるクエリ / 埋まらないクエリ / 無一致 / fuzzy のみ一致 /
    // 複数語 を混ぜる。
    for query in [
        "project",       // 部分一致が大量 (1 巡目で埋まる)
        "pro",           //
        "p",             //
        "zebra",         // 40 件のみ (閾値未満 → 2 巡目へ)
        "zebra archive", // 複数語
        "prjctfldr",     // fuzzy でしか一致しない
        "pjf",           // fuzzy (短い)
        "zzqqxx",        // 無一致
        "project 42",    // 複数語 (片方が数字)
        "folder 7",      //
        "",              // 空クエリ
    ] {
        let optimized = search_entries_cached_multi_for_test(&entries, &lower, query, &ranking);
        let reference = reference_search(&entries, &lower, query, &ranking);
        assert_eq!(
            optimized.len(),
            reference.len(),
            "query={query:?} で件数が変わった"
        );
        for (index, (a, b)) in optimized.iter().zip(&reference).enumerate() {
            assert_eq!(
                a.name, b.name,
                "query={query:?} の {index} 番目で並びが変わった"
            );
        }
    }

    // 上の突き合わせが「枝刈りが起きない条件ばかり」で通っていないことを
    // 確かめる。"project" は tier2/3 で 200 件一致するので閾値 (64) を
    // 超え、1 巡目で打ち切られる経路に入る。
    let cheap_hits = entries
        .iter()
        .filter(|entry| entry.name.to_lowercase().contains("project"))
        .count();
    assert!(
        cheap_hits >= super::super::rank::fuzzy_skip_threshold(),
        "枝刈り経路を通る前提が崩れている: cheap_hits={cheap_hits}"
    );
}

/// 常に fuzzy を評価する参照実装 (枝刈りなし)。
fn reference_search<'a>(
    entries: &'a [Entry],
    lower: &[super::super::search::LowerKeys],
    query: &str,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let _ = lower;
    // `search_entries` は常に fuzzy を評価する経路 (Fuzzy::Include 固定)。
    super::super::search_entries(entries, query, true, ranking)
}

fn search_entries_cached_multi_for_test<'a>(
    entries: &'a [Entry],
    lower: &[super::super::search::LowerKeys],
    query: &str,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    super::super::rank::bench_cached_multi(&[(entries, lower)], query, true, ranking)
}
