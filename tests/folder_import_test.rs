//! フォルダ構造を設定項目へ一括取り込みするロジックのテスト。

use std::path::PathBuf;

use waypoint::config::Item;
use waypoint::folder_import::{FolderNode, scan};

fn test_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "waypoint-folder-import-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn scan_respects_depth_and_sorts_folders() {
    let root = test_root("depth");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("B").join("Nested")).unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("ignored.txt"), "file").unwrap();

    let node = scan(&root, 1).unwrap();

    assert_eq!(node.children.len(), 2);
    assert_eq!(node.children[0].name, "a");
    assert_eq!(node.children[1].name, "B");
    assert!(node.children[1].children.is_empty());

    let deeper = scan(&root, 2).unwrap();
    assert_eq!(deeper.children[1].children[0].name, "Nested");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn excluded_nodes_are_not_converted() {
    let root = test_root("exclude");
    let mut node = FolderNode {
        name: "Root".to_string(),
        path: root.clone(),
        included: true,
        children: vec![FolderNode {
            name: "Child".to_string(),
            path: root.join("Child"),
            included: false,
            children: Vec::new(),
        }],
    };

    assert!(matches!(node.to_item(), Some(Item::Folder { .. })));
    node.children[0].included = true;
    let Some(Item::Submenu { items, .. }) = node.to_item() else {
        panic!("children should create a submenu");
    };
    assert_eq!(items.len(), 2);
}
