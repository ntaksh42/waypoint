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

use crate::browser_tabs::{Browser, Tab};
use crate::window_instance::WindowMarker;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};
use windows::core::w;

struct TestWindow(HWND);

impl TestWindow {
    fn new() -> Self {
        Self(unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Alias test"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0).unwrap() }
    }
}

#[test]
fn window_alias_follows_the_same_window_and_expires_on_close() {
    let mut index = index();
    let window = TestWindow::new();
    let duplicate = TestWindow::new();
    let mut entry = index.windows[0].clone();
    entry.action = Action::FocusWindow(window.0.0 as isize);
    let mut same_title = entry.clone();
    same_title.action = Action::FocusWindow(duplicate.0.0 as isize);
    index.windows = vec![same_title, entry.clone()];
    let live = index.aliases.live.begin(&entry).unwrap().unwrap();
    index.aliases.live.set(live, vec!["windowprobe".into()]);

    index.windows[1].name = "Renamed window".into();
    let found = index.apply_aliases("windowprobe", vec![index.windows[1].clone()], 24);
    assert_eq!(names(&found), ["Renamed window"]);
    assert_eq!(found[0].action, entry.action);

    drop(window);
    assert!(
        index
            .apply_aliases("windowprobe", Vec::new(), 24)
            .is_empty()
    );
    assert!(
        index
            .aliases
            .live
            .window_selection_is_stale(&entry, "windowprobe")
    );
    index.prune_live_aliases();
    assert!(index.aliases.is_empty());
    assert_eq!(index.windows.len(), 1, "閉じた対象が通常検索に残らない");
}

#[test]
fn tab_alias_uses_the_current_tab_in_the_same_connection_only() {
    let mut index = index();
    let host = TestWindow::new();
    let first = WindowMarker::attach(host.0).unwrap();
    let mut tab = Tab {
        source: Some(first.0),
        id: 7,
        window_id: 1,
        title: "Original tab".into(),
        url: "https://example.test/one".into(),
    };
    index.set_browser_tabs(&[(Browser::Edge, tab.clone())]);
    let live = index.aliases.live.begin(&index.tabs[0]).unwrap().unwrap();
    index.aliases.live.set(live, vec!["tabprobe".into()]);

    tab.window_id = 3;
    tab.title = "Moved tab".into();
    tab.url = "https://example.test/two".into();
    index.set_browser_tabs(&[(Browser::Edge, tab.clone())]);
    index.prune_live_aliases();
    let found = search_with_aliases(&index, "tabprobe");
    assert_eq!(names(&found), ["Moved tab"]);
    assert!(matches!(&found[0].action, Action::FocusBrowserTab(target)
        if target.focus_only && target.window_id == 3 && target.source == Some(first.0)));

    // HWND とタブ ID が同じでも接続が変わったら別の対象として扱う。
    let second = WindowMarker::attach(host.0).unwrap();
    tab.source = Some(second.0);
    index.set_browser_tabs(&[(Browser::Edge, tab)]);
    assert!(search_with_aliases(&index, "tabprobe").is_empty());
    index.prune_live_aliases();
    assert!(index.aliases.is_empty());
}

#[test]
fn live_aliases_never_enter_the_alias_file() {
    let mut index = index();
    let host = TestWindow::new();
    let marker = WindowMarker::attach(host.0).unwrap();
    index.set_browser_tabs(&[(
        Browser::Edge,
        Tab {
            source: Some(marker.0),
            id: 7,
            window_id: 1,
            title: "Unstored tab title".into(),
            url: "https://example.test/unpersisted".into(),
        },
    )]);
    let persistent = serde_json::to_vec(&index.aliases).unwrap();
    let target = index.aliases.live.begin(&index.tabs[0]).unwrap().unwrap();
    index.aliases.live.set(target, vec!["temporary".into()]);
    assert!(!index.aliases.is_empty());
    assert_eq!(serde_json::to_vec(&index.aliases).unwrap(), persistent);

    index.set_browser_tabs(&[]);
    index.prune_live_aliases();
    assert!(index.aliases.is_empty());
}
