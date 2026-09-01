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
            show_branch: false,
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
                && !file_type.is_symlink()
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
