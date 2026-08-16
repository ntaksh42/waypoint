//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch_history::Ranking;

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
    /// showBranch が真の Folder 項目のみ。構築時にまとめて読む (FR-2.15) 。
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: Vec<Entry>,
    bookmarks: Vec<Entry>,
    windows: Vec<Entry>,
    apps: Vec<Entry>,
    search_paths: bool,
    ranking: Ranking,
}

impl Index {
    pub fn build(config: &Config, dynamic: &Menus) -> Self {
        let mut entries = Vec::new();
        collect_items(
            &config.items,
            &config.variables,
            &mut Vec::new(),
            false,
            &mut entries,
        );

        let settings = &config.settings.quick_launch;
        if settings.include_recent_folders {
            entries.extend(dynamic.recent_folders.iter().map(|item| Entry {
                name: item.name.clone(),
                breadcrumb: "Recent Folders".to_string(),
                path: item.path.clone(),
                action: Action::OpenFolder(OpenMode::NewWindow),
                branch: None,
            }));
        }
        if settings.include_frequent_folders {
            entries.extend(dynamic.frequent_folders.iter().map(|item| Entry {
                name: item.name.clone(),
                breadcrumb: "Frequent Folders".to_string(),
                path: item.path.clone(),
                action: Action::OpenFolder(OpenMode::NewWindow),
                branch: None,
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
                    branch: None,
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
                    branch: None,
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
                    branch: None,
                })
                .collect()
        } else {
            Vec::new()
        };

        Self {
            entries: dedup_by_path(entries),
            bookmarks,
            windows,
            apps,
            search_paths: settings.search_paths,
            ranking: Ranking::load(),
        }
    }

    /// `b ` / `w ` / `a ` で始まる入力の間は、それぞれブックマーク・
    /// Open Windows・アプリだけを検索する (FR-9.13 / FR-9.15 / FR-9.14)。
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        if let Some(rest) = query.strip_prefix(BOOKMARK_PREFIX) {
            return search_entries(&self.bookmarks, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(WINDOW_PREFIX) {
            return search_entries(&self.windows, rest, false, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(APPS_PREFIX) {
            return search_entries(&self.apps, rest, false, &self.ranking);
        }
        search_entries(&self.entries, query, self.search_paths, &self.ranking)
    }
}

fn search_entries<'a>(
    entries: &'a [Entry],
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matches: Vec<(u8, (u64, u64), usize, &Entry)> = entries
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
            scores.map(|scores| {
                (
                    scores.into_iter().max().unwrap_or(0),
                    ranking.rank(entry),
                    order,
                    entry,
                )
            })
        })
        .collect();
    // 文字列一致の質を最優先し、同点内では使用頻度・最近使った順で並べる
    matches.sort_by_key(|(score, usage, order, _)| (*score, *usage, *order));
    matches.into_iter().map(|(_, _, _, entry)| entry).collect()
}

/// 同じパスを指す項目 (config の Folder / Recent Folders / Frequent
/// Folders など、出所違いで同一フォルダが複数登録され得る) を 1 件へ
/// たたむ。先に追加された方を残すので、config.items を優先し
/// Recent > Frequent の順にフォールバックする (呼び出し側の追加順)。
fn dedup_by_path(entries: Vec<Entry>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|entry| {
            if entry.path.is_empty() {
                return true;
            }
            seen.insert(entry.path.to_lowercase())
        })
        .collect()
}

/// `inherited_show_branch` は祖先 Submenu の showBranch が真だったか。
/// 真なら配下の Folder は自身の showBranch を問わずブランチ名を持たせる
/// (FR-2.14、menu.rs の build_level と同じ継承規則) 。
fn collect_items(
    items: &[Item],
    variables: &std::collections::BTreeMap<String, String>,
    parents: &mut Vec<String>,
    inherited_show_branch: bool,
    entries: &mut Vec<Entry>,
) {
    for item in items {
        match item {
            Item::Folder {
                name,
                path,
                open,
                show_branch,
                ..
            } => {
                if let Some(path) = crate::config::expand(path, variables) {
                    // ブランチ名の付与は構築時に済ませる。表示経路では読まない (FR-2.15)
                    let branch = (inherited_show_branch || *show_branch)
                        .then(|| crate::git::branch_of(&path))
                        .flatten();
                    entries.push(Entry {
                        name: name.clone(),
                        breadcrumb: parents.join(" > "),
                        path,
                        action: Action::OpenFolder(open.unwrap_or_default()),
                        branch,
                    });
                }
            }
            Item::File { name, path, .. } => {
                if let Some(path) = crate::config::expand(path, variables) {
                    entries.push(Entry {
                        name: name.clone(),
                        breadcrumb: parents.join(" > "),
                        path,
                        action: Action::OpenWithDefaultHandler,
                        branch: None,
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
                        branch: None,
                    });
                }
            }
            Item::Shell { name, target } => {
                entries.push(Entry {
                    name: name.clone(),
                    breadcrumb: parents.join(" > "),
                    path: target.clone(),
                    action: Action::OpenWithDefaultHandler,
                    branch: None,
                });
            }
            Item::Submenu {
                name,
                items,
                show_branch,
            } => {
                parents.push(name.clone());
                collect_items(
                    items,
                    variables,
                    parents,
                    inherited_show_branch || *show_branch,
                    entries,
                );
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
                    branch: None,
                },
                Entry {
                    name: "Waypoint docs".into(),
                    breadcrumb: "Projects".into(),
                    path: r"E:\waypoint\docs".into(),
                    action: Action::OpenFolder(OpenMode::NewWindow),
                    branch: None,
                },
                Entry {
                    name: "Old waypoint".into(),
                    breadcrumb: "Archive".into(),
                    path: r"E:\archive\waypoint".into(),
                    action: Action::OpenFolder(OpenMode::NewWindow),
                    branch: None,
                },
            ],
            bookmarks: vec![
                Entry {
                    name: "GitHub".into(),
                    breadcrumb: "Work".into(),
                    path: "https://github.com/".into(),
                    action: Action::OpenUrl("https://github.com/".into()),
                    branch: None,
                },
                Entry {
                    name: "Example".into(),
                    breadcrumb: String::new(),
                    path: "https://example.com/".into(),
                    action: Action::OpenUrl("https://example.com/".into()),
                    branch: None,
                },
            ],
            windows: vec![Entry {
                name: "waypoint - Notepad".into(),
                breadcrumb: "Open Windows".into(),
                path: String::new(),
                action: Action::FocusWindow(12345),
                branch: None,
            }],
            apps: vec![Entry {
                name: "Visual Studio Code".into(),
                breadcrumb: String::new(),
                path:
                    r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk"
                        .into(),
                action: Action::LaunchApp,
                branch: None,
            }],
            search_paths: false,
            ranking: Ranking::default(),
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
    fn previously_selected_entry_ranks_before_same_score_siblings() {
        // 両方とも prefix 一致で同スコアになる 2 件。並び順 (order) だけなら
        // Alpha が先に出るはずだが、Beta の選択履歴があれば逆転する
        let alpha = Entry {
            name: "Alpha Tools".into(),
            breadcrumb: String::new(),
            path: r"C:\Alpha".into(),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        };
        let beta = Entry {
            name: "Alpha Utils".into(),
            breadcrumb: String::new(),
            path: r"C:\Beta".into(),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        };
        let mut idx = Index {
            entries: vec![alpha, beta.clone()],
            ..Index::default()
        };
        idx.ranking = Ranking::default().with_selection(&beta, 3, 100);

        let found = idx.search("alpha");
        assert_eq!(
            found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["Alpha Utils", "Alpha Tools"]
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
    fn shell_items_are_indexed_and_open_with_default_handler() {
        let config = Config {
            items: vec![Item::Shell {
                name: "This PC".to_string(),
                target: "shell:MyComputerFolder".to_string(),
            }],
            ..Config::default()
        };
        let index = Index::build(&config, &Menus::default());
        let found = index.search("this pc");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "shell:MyComputerFolder");
        assert_eq!(found[0].action, Action::OpenWithDefaultHandler);
    }

    #[test]
    fn file_items_are_indexed_and_open_with_default_handler() {
        let config = Config {
            items: vec![Item::File {
                name: "Notes".to_string(),
                path: r"E:\notes.txt".to_string(),
                icon: None,
            }],
            ..Config::default()
        };
        let index = Index::build(&config, &Menus::default());
        let found = index.search("notes");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, r"E:\notes.txt");
        assert_eq!(found[0].action, Action::OpenWithDefaultHandler);
    }

    #[test]
    fn same_path_from_config_and_recent_and_frequent_folds_into_one() {
        use crate::dynamic::PathEntry;

        let config = Config {
            items: vec![Item::Folder {
                name: "DevDeck".to_string(),
                path: r"E:\DevDeck".to_string(),
                open: None,
                icon: None,
                show_branch: false,
            }],
            ..Config::default()
        };
        let dynamic = Menus {
            recent_folders: vec![PathEntry {
                name: "DevDeck".to_string(),
                path: r"E:\DevDeck".to_string(),
            }],
            frequent_folders: vec![PathEntry {
                name: "DevDeck".to_string(),
                path: r"e:\devdeck".to_string(), // 大文字小文字違いでも同一視する
            }],
            ..Menus::default()
        };
        let index = Index::build(&config, &dynamic);
        let found = index.search("devdeck");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].breadcrumb, ""); // config.items 直下 = breadcrumb なし
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

    /// showBranch が真の Folder は、このリポジトリ自身を指せば
    /// Entry::branch にブランチ名が入る (FR-2.14 相当) 。
    #[test]
    fn folder_with_show_branch_carries_branch_name() {
        let config = Config {
            items: vec![Item::Folder {
                name: "waypoint".to_string(),
                path: env!("CARGO_MANIFEST_DIR").to_string(),
                open: None,
                icon: None,
                show_branch: true,
            }],
            ..Config::default()
        };
        let index = Index::build(&config, &Menus::default());
        let found = index.search("waypoint");
        assert_eq!(found.len(), 1);
        assert!(found[0].branch.is_some());
    }

    /// showBranch が偽なら、同じパスでもブランチ名を読まない (オプトイン) 。
    #[test]
    fn folder_without_show_branch_has_no_branch() {
        let config = Config {
            items: vec![Item::Folder {
                name: "waypoint".to_string(),
                path: env!("CARGO_MANIFEST_DIR").to_string(),
                open: None,
                icon: None,
                show_branch: false,
            }],
            ..Config::default()
        };
        let index = Index::build(&config, &Menus::default());
        let found = index.search("waypoint");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].branch, None);
    }

    /// Submenu の showBranch は配下の Folder (自身は showBranch=false) へ
    /// 継承される。
    #[test]
    fn submenu_show_branch_is_inherited_by_child_folders() {
        let config = Config {
            items: vec![Item::Submenu {
                name: "Repos".to_string(),
                items: vec![Item::Folder {
                    name: "waypoint".to_string(),
                    path: env!("CARGO_MANIFEST_DIR").to_string(),
                    open: None,
                    icon: None,
                    show_branch: false,
                }],
                show_branch: true,
            }],
            ..Config::default()
        };
        let index = Index::build(&config, &Menus::default());
        let found = index.search("waypoint");
        assert_eq!(found.len(), 1);
        assert!(found[0].branch.is_some());
    }
}
