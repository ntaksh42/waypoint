use super::super::*;

/// 文字が順序通りに (連続でなくても) 含まれていれば Fuzzy でヒットする。
#[test]
fn fuzzy_subsequence_matches_scattered_letters() {
    let entries = vec![Entry {
        name: "Waypoint docs".into(),
        breadcrumb: String::new(),
        path: r"E:\waypoint\docs".into(),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }];
    let index = Index {
        entries,
        ..Index::default()
    };
    // "wdc" は連続一致でも部分一致でもないが、Waypoint docs の頭文字を
    // 順に拾える (fzf 的な Fuzzy)
    let found = index.search("wdc");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Waypoint docs");
}

/// Fuzzy マッチは通常の完全一致 / 前方一致 / 部分一致より下位に位置する。
#[test]
fn fuzzy_match_ranks_below_substring_match() {
    let entries = vec![
        Entry {
            name: "Weird Document Cache".into(),
            breadcrumb: String::new(),
            path: r"C:\wdc".into(),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        },
        Entry {
            name: "wdc exact".into(),
            breadcrumb: String::new(),
            path: r"C:\exact".into(),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        },
    ];
    let index = Index {
        entries,
        ..Index::default()
    };
    let found = index.search("wdc");
    assert_eq!(
        found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["wdc exact", "Weird Document Cache"]
    );
}

/// ひらがな/カタカナのみの項目名は、ローマ字入力でも検索できる (音マッチ)。
#[test]
fn romaji_query_matches_kana_name() {
    let entries = vec![Entry {
        name: "かいはつしつ".into(),
        breadcrumb: String::new(),
        path: r"E:\dev".into(),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }];
    let index = Index {
        entries,
        ..Index::default()
    };
    let found = index.search("kaihatsu");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "かいはつしつ");
}

/// 漢字が混じる項目名は音マッチの対象外 (読みが一意に定まらないため)。
#[test]
fn romaji_query_does_not_match_kanji_name() {
    let entries = vec![Entry {
        name: "開発室".into(),
        breadcrumb: String::new(),
        path: r"E:\dev".into(),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }];
    let index = Index {
        entries,
        ..Index::default()
    };
    assert!(index.search("kaihatsu").is_empty());
}
