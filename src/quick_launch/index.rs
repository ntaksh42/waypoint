//! `Index::build` とその構築ヘルパー。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch_history::Ranking;

use super::azure::{AzureIndexed, azure_candidate_entry};
use super::search::dedup_by_path;
use super::{Action, Entry, Index};

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
                .all_windows
                .iter()
                .map(|window| Entry {
                    name: window.title.clone(),
                    // プロセス名も breadcrumb に含めて検索対象にする。
                    // タイトルにアプリ名が出ないウィンドウも `w chrome` で探せる。
                    breadcrumb: if window.process_name.is_empty() {
                        "Open Windows".to_string()
                    } else {
                        format!("Open Windows — {}", window.process_name)
                    },
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

        let history = if settings.include_browser_history {
            crate::browser_history::scan()
                .into_iter()
                .map(|visit| Entry {
                    name: visit.title,
                    breadcrumb: format!("{} History", visit.browser),
                    path: visit.url.clone(),
                    action: Action::OpenUrl(visit.url),
                    branch: None,
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut azure_candidates = if settings.azure_devops.enabled {
            crate::azure_devops::project_candidates(&settings.azure_devops)
        } else {
            Vec::new()
        };
        azure_candidates.extend(crate::azure_devops::cached_candidates(
            &settings.azure_devops,
        ));
        // 優先度を最優先しつつ、同一プロジェクト内では日常的に開く Active PR と
        // 失敗した Pipeline を先頭へ置く。通常の使用履歴ランキングも後段で効く。
        azure_candidates.sort_by_key(|candidate| {
            let urgency = match (&candidate.kind, candidate.status.as_str()) {
                (crate::azure_devops::Kind::PullRequest, status)
                    if status.eq_ignore_ascii_case("active") =>
                {
                    0
                }
                (crate::azure_devops::Kind::Pipeline, status)
                    if status.eq_ignore_ascii_case("failed") =>
                {
                    1
                }
                _ => 2,
            };
            (candidate.priority, urgency)
        });
        let azure = azure_candidates
            .into_iter()
            .map(|candidate| AzureIndexed {
                entry: azure_candidate_entry(candidate.clone()),
                kind: candidate.kind,
                status: candidate.status,
                is_mine: candidate.is_mine,
            })
            .collect();
        let azure_work_items =
            crate::azure_devops::cached_work_item_candidates(&settings.azure_devops)
                .into_iter()
                .map(azure_candidate_entry)
                .collect();

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
            history,
            azure,
            azure_work_items,
            windows,
            apps,
            search_paths: settings.search_paths,
            ranking: Ranking::load(),
        }
    }
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
