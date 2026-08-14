//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;

/// 検索結果を選んだときに行うアクション。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// フォルダを開く (新規ウィンドウ / 既存ウィンドウの再利用)。
    OpenFolder(OpenMode),
    /// 既に開いているウィンドウにフォーカスを移す。
    FocusWindow(isize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub breadcrumb: String,
    pub path: String,
    pub action: Action,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: Vec<Entry>,
    search_paths: bool,
}

impl Index {
    pub fn build(config: &Config, dynamic: &Menus) -> Self {
        let mut entries = Vec::new();
        collect_items(
            &config.items,
            &config.variables,
            &mut Vec::new(),
            &mut entries,
        );

        let settings = &config.settings.quick_launch;
        if settings.include_recent_folders {
            entries.extend(dynamic.recent_folders.iter().map(|item| Entry {
                name: item.name.clone(),
                breadcrumb: "Recent Folders".to_string(),
                path: item.path.clone(),
                action: Action::OpenFolder(OpenMode::NewWindow),
            }));
        }
        if settings.include_frequent_folders {
            entries.extend(dynamic.frequent_folders.iter().map(|item| Entry {
                name: item.name.clone(),
                breadcrumb: "Frequent Folders".to_string(),
                path: item.path.clone(),
                action: Action::OpenFolder(OpenMode::NewWindow),
            }));
        }
        if settings.include_open_windows {
            entries.extend(dynamic.current_windows.iter().map(|window| Entry {
                name: window.title.clone(),
                breadcrumb: "Open Windows".to_string(),
                path: String::new(),
                action: Action::FocusWindow(window.hwnd),
            }));
        }

        Self {
            entries,
            search_paths: settings.search_paths,
        }
    }

    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut matches: Vec<(u8, usize, &Entry)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(order, entry)| {
                let name = entry.name.to_lowercase();
                let breadcrumb = entry.breadcrumb.to_lowercase();
                let path = self.search_paths.then(|| entry.path.to_lowercase());
                let scores: Option<Vec<u8>> = terms
                    .iter()
                    .map(|term| match_score(&name, &breadcrumb, path.as_deref(), term))
                    .collect();
                scores.map(|scores| (scores.into_iter().max().unwrap_or(0), order, entry))
            })
            .collect();
        matches.sort_by_key(|(score, order, _)| (*score, *order));
        matches.into_iter().map(|(_, _, entry)| entry).collect()
    }
}

fn collect_items(
    items: &[Item],
    variables: &std::collections::BTreeMap<String, String>,
    parents: &mut Vec<String>,
    entries: &mut Vec<Entry>,
) {
    for item in items {
        match item {
            Item::Folder {
                name, path, open, ..
            } => {
                if let Some(path) = crate::config::expand(path, variables) {
                    entries.push(Entry {
                        name: name.clone(),
                        breadcrumb: parents.join(" > "),
                        path,
                        action: Action::OpenFolder(open.unwrap_or_default()),
                    });
                }
            }
            Item::SpecialFolder {
                name,
                known_folder,
                open,
            } => {
                if let Some(path) = crate::known_folder::resolve(known_folder) {
                    entries.push(Entry {
                        name: name.clone(),
                        breadcrumb: parents.join(" > "),
                        path,
                        action: Action::OpenFolder(open.unwrap_or_default()),
                    });
                }
            }
            Item::Submenu { name, items } => {
                parents.push(name.clone());
                collect_items(items, variables, parents, entries);
                parents.pop();
            }
            Item::Separator { .. } => {}
        }
    }
}

fn match_score(name: &str, breadcrumb: &str, path: Option<&str>, term: &str) -> Option<u8> {
    if name == term {
        Some(0)
    } else if name.starts_with(term) {
        Some(1)
    } else if name
        .match_indices(term)
        .any(|(index, _)| index == 0 || name[..index].ends_with([' ', '-', '_', '.']))
    {
        Some(2)
    } else if name.contains(term) {
        Some(3)
    } else if breadcrumb.contains(term) {
        Some(4)
    } else if path.is_some_and(|path| path.contains(term)) {
        Some(5)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index {
            entries: vec![
                Entry {
                    name: "Release".into(),
                    breadcrumb: "Projects > waypoint".into(),
                    path: r"E:\waypoint\target\release".into(),
                    action: Action::OpenFolder(OpenMode::Reuse),
                },
                Entry {
                    name: "Waypoint docs".into(),
                    breadcrumb: "Projects".into(),
                    path: r"E:\waypoint\docs".into(),
                    action: Action::OpenFolder(OpenMode::NewWindow),
                },
                Entry {
                    name: "Old waypoint".into(),
                    breadcrumb: "Archive".into(),
                    path: r"E:\archive\waypoint".into(),
                    action: Action::OpenFolder(OpenMode::NewWindow),
                },
            ],
            search_paths: false,
        }
    }

    #[test]
    fn all_terms_must_match_name_or_breadcrumb() {
        let index = index();
        let found = index.search("way rel");
        assert_eq!(
            found
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["Release"]
        );
    }

    #[test]
    fn exact_and_prefix_matches_rank_before_substrings() {
        let index = index();
        let found = index.search("waypoint");
        assert_eq!(
            found
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["Waypoint docs", "Old waypoint", "Release"]
        );
    }

    #[test]
    fn path_search_is_opt_in() {
        let mut index = index();
        assert!(index.search("target").is_empty());
        index.search_paths = true;
        assert_eq!(index.search("target")[0].name, "Release");
    }

    #[test]
    fn open_window_entries_focus_instead_of_opening_a_folder() {
        let mut index = index();
        index.entries.push(Entry {
            name: "waypoint - Notepad".into(),
            breadcrumb: "Open Windows".into(),
            path: String::new(),
            action: Action::FocusWindow(12345),
        });
        let found = index.search("notepad");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, Action::FocusWindow(12345));
    }
}
