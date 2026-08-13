//! フォルダ構造を設定項目へ一括取り込みするための純ロジック。

use std::io;
use std::path::{Path, PathBuf};

use crate::config::Item;

#[derive(Debug, Clone, PartialEq)]
pub struct FolderNode {
    pub name: String,
    pub path: PathBuf,
    pub included: bool,
    pub children: Vec<FolderNode>,
}

impl FolderNode {
    /// 選択されたノードを既存のメニュー項目へ変換する。
    pub fn to_item(&self) -> Option<Item> {
        if !self.included {
            return None;
        }

        let children: Vec<Item> = self.children.iter().filter_map(Self::to_item).collect();
        if children.is_empty() {
            return Some(folder_item(self.name.clone(), &self.path));
        }

        let mut items = Vec::with_capacity(children.len() + 1);
        items.push(folder_item("Open this folder".to_string(), &self.path));
        items.extend(children);
        Some(Item::Submenu {
            name: self.name.clone(),
            items,
        })
    }

    pub fn included_count(&self) -> usize {
        if !self.included {
            return 0;
        }
        1 + self
            .children
            .iter()
            .map(Self::included_count)
            .sum::<usize>()
    }
}

/// `root` を 0 として、指定した深さまでサブフォルダを読み込む。
pub fn scan(root: &Path, max_depth: usize) -> io::Result<FolderNode> {
    if root.as_os_str().is_empty() || !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The selected path is not a folder.",
        ));
    }
    scan_node(root, max_depth)
}

fn scan_node(path: &Path, remaining_depth: usize) -> io::Result<FolderNode> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut children = Vec::new();

    if remaining_depth > 0 {
        for entry in std::fs::read_dir(path)? {
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && let Ok(child) = scan_node(&entry.path(), remaining_depth - 1)
            {
                children.push(child);
            }
        }
        children.sort_by_cached_key(|node| node.name.to_lowercase());
    }

    Ok(FolderNode {
        name,
        path: path.to_path_buf(),
        included: true,
        children,
    })
}

fn folder_item(name: String, path: &Path) -> Item {
    Item::Folder {
        name,
        path: path.display().to_string(),
        open: None,
        icon: None,
        show_branch: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
