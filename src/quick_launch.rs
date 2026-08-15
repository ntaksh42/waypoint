//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;

/// ブックマーク検索モードに入るプレフィックス (末尾の半角スペース込み)。
const BOOKMARK_PREFIX: &str = "b ";
/// Open Windows 検索モードに入るプレフィックス (末尾の半角スペース込み)。
const WINDOW_PREFIX: &str = "w ";
/// Everything 検索モードに入るプレフィックス (末尾の半角スペース込み)。
///
/// Everything の検索は Win32 IPC を挟む非同期処理のため、他のプレフィックス
/// と違って `Index::search` の同期モデルには乗らない。判定だけここに置き、
/// クエリの発行と結果の保持は `quick_launch_window.rs` 側が持つ。
pub const EVERYTHING_PREFIX: &str = "f ";
/// アプリ検索モードに入るプレフィックス (末尾の半角スペース込み)。
const APPS_PREFIX: &str = "a ";

/// 入力がいずれかのプレフィックスモードに入っていれば、表示用の短いラベルを返す。
/// 描画側 (`quick_launch_window.rs`) が検索窓にモードバッジを出すために使う。
pub fn prefix_badge(query: &str) -> Option<&'static str> {
    if query.starts_with(BOOKMARK_PREFIX) {
        Some("BOOKMARKS")
    } else if query.starts_with(WINDOW_PREFIX) {
        Some("WINDOWS")
    } else if query.starts_with(APPS_PREFIX) {
        Some("APPS")
    } else if query.starts_with(EVERYTHING_PREFIX) {
        Some("FILES")
    } else {
        None
    }
}

/// 検索結果を選んだときに行うアクション。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// フォルダを開く (新規ウィンドウ / 既存ウィンドウの再利用)。
    OpenFolder(OpenMode),
    /// 既に開いているウィンドウにフォーカスを移す。
    FocusWindow(isize),
    /// 既定のブラウザで URL を開く。
    OpenUrl(String),
    /// Windows の既定ハンドラーでファイル / フォルダを開く (Everything 結果用)。
    /// `OpenFolder` と違い newWindow / reuse の区別を持たない。
    OpenWithDefaultHandler,
    /// スタートメニューのショートカットを起動する。
    LaunchApp,
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
    bookmarks: Vec<Entry>,
    windows: Vec<Entry>,
    apps: Vec<Entry>,
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
        let windows = if settings.include_open_windows {
            dynamic
                .current_windows
                .iter()
                .map(|window| Entry {
                    name: window.title.clone(),
                    breadcrumb: "Open Windows".to_string(),
                    path: String::new(),
                    action: Action::FocusWindow(window.hwnd),
                })
                .collect()
        } else {
            Vec::new()
        };

        let bookmarks = if settings.include_bookmarks {
            crate::bookmarks::scan()
                .into_iter()
                .map(|bookmark| Entry {
                    name: bookmark.name,
                    breadcrumb: bookmark.breadcrumb,
                    path: bookmark.url.clone(),
                    action: Action::OpenUrl(bookmark.url),
                })
                .collect()
        } else {
            Vec::new()
        };

        let apps = if settings.include_apps {
            crate::apps::scan()
                .into_iter()
                .map(|app| Entry {
                    name: app.name,
                    breadcrumb: String::new(),
                    path: app.shortcut_path,
                    action: Action::LaunchApp,
                })
                .collect()
        } else {
            Vec::new()
        };

        Self {
            entries,
            bookmarks,
            windows,
            apps,
            search_paths: settings.search_paths,
        }
    }

    /// `b ` / `w ` / `a ` で始まる入力の間は、それぞれブックマーク・
    /// Open Windows・アプリだけを検索する (FR-9.13 / FR-9.15 / FR-9.14)。
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        if let Some(rest) = query.strip_prefix(BOOKMARK_PREFIX) {
            return search_entries(&self.bookmarks, rest, true);
        }
        if let Some(rest) = query.strip_prefix(WINDOW_PREFIX) {
            return search_entries(&self.windows, rest, false);
        }
        if let Some(rest) = query.strip_prefix(APPS_PREFIX) {
            return search_entries(&self.apps, rest, false);
        }
        search_entries(&self.entries, query, self.search_paths)
    }
}

fn search_entries<'a>(entries: &'a [Entry], query: &str, search_paths: bool) -> Vec<&'a Entry> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matches: Vec<(u8, usize, &Entry)> = entries
        .iter()
        .enumerate()
        .filter_map(|(order, entry)| {
            let name = entry.name.to_lowercase();
            let breadcrumb = entry.breadcrumb.to_lowercase();
            let path = search_paths.then(|| entry.path.to_lowercase());
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
            bookmarks: vec![
                Entry {
                    name: "GitHub".into(),
                    breadcrumb: "Work".into(),
                    path: "https://github.com/".into(),
                    action: Action::OpenUrl("https://github.com/".into()),
                },
                Entry {
                    name: "Example".into(),
                    breadcrumb: String::new(),
                    path: "https://example.com/".into(),
                    action: Action::OpenUrl("https://example.com/".into()),
                },
            ],
            windows: vec![Entry {
                name: "waypoint - Notepad".into(),
                breadcrumb: "Open Windows".into(),
                path: String::new(),
                action: Action::FocusWindow(12345),
            }],
            apps: vec![Entry {
                name: "Visual Studio Code".into(),
                breadcrumb: String::new(),
                path:
                    r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk"
                        .into(),
                action: Action::LaunchApp,
            }],
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
    fn window_prefix_switches_to_window_only_search() {
        let index = index();
        let found = index.search("w notepad");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, Action::FocusWindow(12345));
    }

    #[test]
    fn without_the_window_prefix_open_windows_are_not_searched() {
        let index = index();
        assert!(index.search("notepad").is_empty());
    }

    #[test]
    fn apps_prefix_switches_to_apps_only_search() {
        let index = index();
        let found = index.search("a code");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, Action::LaunchApp);
    }

    #[test]
    fn without_the_apps_prefix_apps_are_not_searched() {
        let index = index();
        assert!(index.search("code").is_empty());
    }

    #[test]
    fn bookmark_prefix_switches_to_bookmark_only_search() {
        let index = index();
        let found = index.search("b git");
        assert_eq!(
            found
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["GitHub"]
        );
        assert_eq!(
            found[0].action,
            Action::OpenUrl("https://github.com/".into())
        );
    }

    #[test]
    fn without_the_bookmark_prefix_bookmarks_are_not_searched() {
        let index = index();
        assert!(index.search("github").is_empty());
    }

    #[test]
    fn bookmark_search_matches_the_url_too() {
        let index = index();
        let found = index.search("b example.com");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Example");
    }

    #[test]
    fn prefix_badge_identifies_each_mode() {
        assert_eq!(prefix_badge("b git"), Some("BOOKMARKS"));
        assert_eq!(prefix_badge("w notepad"), Some("WINDOWS"));
        assert_eq!(prefix_badge("a code"), Some("APPS"));
        assert_eq!(prefix_badge("f cargo.toml"), Some("FILES"));
        assert_eq!(prefix_badge("plain query"), None);
        assert_eq!(prefix_badge(""), None);
    }
}
