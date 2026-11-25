use super::super::*;
use super::fixture::index;

fn search_with_aliases(index: &Index, query: &str) -> Vec<Entry> {
    let results = index.search(query).into_iter().cloned().collect();
    index.apply_aliases(query, results, 24)
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

fn find(index: &Index, name: &str) -> Entry {
    index
        .entries
        .iter()
        .chain(&index.apps)
        .chain(&index.history)
        .find(|entry| entry.name == name)
        .cloned()
        .unwrap()
}

#[test]
fn exact_alias_comes_first_without_duplicates() {
    let mut index = index();
    let vscode = find(&index, "Visual Studio Code");
    index.aliases.set(&vscode, vec!["vs".into()]);
    let found = search_with_aliases(&index, "VS");
    assert_eq!(found[0].name, "Visual Studio Code");
    assert_eq!(
        names(&found)
            .iter()
            .filter(|name| **name == "Visual Studio Code")
            .count(),
        1
    );
}

#[test]
fn alias_prefix_joins_the_name_prefix_tier() {
    let mut index = index();
    // 閲覧履歴はプレフィックス無しでは検索しないが、別名なら出せる
    let history = find(&index, "WayPoint pull request");
    index.aliases.set(&history, vec!["wpr".into()]);
    let plain: Vec<Entry> = index.search("w").into_iter().cloned().collect();
    let name_prefix = plain
        .iter()
        .take_while(|entry| entry.name.to_lowercase().starts_with('w'))
        .count();
    let found = search_with_aliases(&index, "w");
    assert_eq!(found[name_prefix].name, "WayPoint pull request");
    assert_eq!(found.len(), plain.len() + 1);
}

#[test]
fn queries_with_spaces_or_paths_ignore_aliases() {
    let mut index = index();
    let vscode = find(&index, "Visual Studio Code");
    index.aliases.set(&vscode, vec!["vs".into(), "c".into()]);
    for query in ["vs ", "b vs", r"C:\"] {
        let plain: Vec<Entry> = index.search(query).into_iter().cloned().collect();
        assert_eq!(search_with_aliases(&index, query), plain, "{query}");
    }
}

#[test]
fn alias_for_candidate_missing_from_index_uses_saved_entry() {
    let mut index = index();
    let everything = Entry {
        azure: None,
        name: "report.xlsx".into(),
        breadcrumb: r"D:\docs".into(),
        path: r"D:\docs\report.xlsx".into(),
        action: Action::OpenWithDefaultHandler,
        branch: None,
    };
    index.aliases.set(&everything, vec!["rep".into()]);
    let found = search_with_aliases(&index, "rep");
    assert_eq!(found[0], everything);
}

#[test]
fn index_entry_wins_over_saved_snapshot() {
    let mut index = index();
    let mut release = find(&index, "Release");
    release.name = "Old name".into();
    index.aliases.set(&release, vec!["rl".into()]);
    let found = search_with_aliases(&index, "rl");
    assert_eq!(found[0].name, "Release");
    assert_eq!(
        found[0].action,
        Action::OpenFolder(crate::config::OpenMode::Reuse)
    );
}

#[test]
fn shared_alias_orders_targets_by_usage() {
    let mut index = index();
    let release = find(&index, "Release");
    let docs = find(&index, "Waypoint docs");
    index.aliases.set(&release, vec!["w1".into()]);
    index.aliases.set(&docs, vec!["w1".into()]);
    index.ranking = std::mem::take(&mut index.ranking).with_selection(&docs, 5, 10);
    let found = search_with_aliases(&index, "w1");
    assert_eq!(names(&found[..2]), ["Waypoint docs", "Release"]);
}
