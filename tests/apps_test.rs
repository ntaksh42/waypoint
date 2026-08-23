//! スタートメニューのショートカット列挙 (FR-9.14) のテスト。

use std::collections::HashSet;
use std::fs;

use waypoint::apps::collect;

#[test]
fn nested_shortcuts_are_collected_recursively() {
    let dir = std::env::temp_dir().join(format!("waypoint-apps-test-{}", std::process::id()));
    let sub = dir.join("Accessories");
    fs::create_dir_all(&sub).unwrap();
    fs::write(dir.join("Top.lnk"), b"").unwrap();
    fs::write(sub.join("Nested.lnk"), b"").unwrap();
    fs::write(sub.join("readme.txt"), b"").unwrap();

    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    collect(&dir, &mut apps, &mut seen);

    let mut names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Nested", "Top"]);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn duplicate_names_across_roots_keep_the_first_seen() {
    let dir = std::env::temp_dir().join(format!("waypoint-apps-dup-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("App.lnk"), b"").unwrap();

    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    collect(&dir, &mut apps, &mut seen);
    collect(&dir, &mut apps, &mut seen);

    assert_eq!(apps.len(), 1);

    fs::remove_dir_all(&dir).unwrap();
}
