//! `Index::build` とその構築ヘルパー。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch_history::Ranking;

use super::azure::{AzureIndexed, azure_candidate_entry};
use super::search::dedup_by_path;
use super::{Action, Entry, Index};

impl Index {
    pub fn build(config: &Config, dynamic: &Menus) -> Self {
        let mut config_entries = Vec::new();
        collect_items(
            &config.items,
            &config.variables,
            &mut Vec::new(),
            false,
            &mut config_entries,
        );

        let settings = &config.settings.quick_launch;
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

        let azure = azure_indexed(settings);
        let azure_work_items = azure_work_item_entries(settings);

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

        let bookmarks_lower = super::search::LowerKeys::build_for(&bookmarks);
        let history_lower = super::search::LowerKeys::build_for(&history);
        let apps_lower = super::search::LowerKeys::build_for(&apps);
        let azure_work_items_lower = super::search::LowerKeys::build_for(&azure_work_items);

        let (entries, entries_lower, windows, windows_lower) =
            dynamic_entries(&config_entries, settings, dynamic);

        Self {
            config_entries,
            entries,
            entries_lower,
            bookmarks,
            bookmarks_lower,
            history,
            history_lower,
            azure,
            azure_work_items,
            azure_work_items_lower,
            windows,
            windows_lower,
            apps,
            apps_lower,
            tabs: Vec::new(),
            tabs_lower: Vec::new(),
            search_paths: settings.search_paths,
            ranking: Ranking::load(),
        }
    }

    /// Recent/Frequent Folders と開いているウィンドウの一覧だけを差し替える。
    ///
    /// `apps` / `bookmarks` / `history` / `azure*` はスタートメニューの COM 解決や
    /// SQLite クエリを伴い、メニューを閉じるたびに作り直すには重すぎる
    /// (実測: スタートメニューの規模次第で数十 ms) 。実際に変わるのは
    /// Recent/Frequent Folders と開いているウィンドウだけなので、そこだけ
    /// 差し替えて残りは保持する (`refresh_dynamic` からの呼び出し用、
    /// `tray::actions::handle_dynamic_refreshed` 参照)。
    pub fn refresh_dynamic(&mut self, config: &Config, dynamic: &Menus) {
        let settings = &config.settings.quick_launch;
        let (entries, entries_lower, windows, windows_lower) =
            dynamic_entries(&self.config_entries, settings, dynamic);
        self.entries = entries;
        self.entries_lower = entries_lower;
        self.windows = windows;
        self.windows_lower = windows_lower;
    }

    /// config 由来の候補 (`config_entries`) と、それを含む `entries` /
    /// `windows` を組み直す。apps / bookmarks / history / azure* は保持する。
    ///
    /// Quick Launch からのお気に入り登録 (`Ctrl+Shift+Enter`、FR-9.7) のように
    /// config だけが変わったときに使う。ここでフル `Index::build` を呼ぶと、
    /// 変わっていないスタートメニューの再スキャン (実測で数十 ms) が
    /// ユーザー操作のたびに UI スレッドで走る。
    pub fn refresh_config_items(&mut self, config: &Config, dynamic: &Menus) {
        let mut config_entries = Vec::new();
        collect_items(
            &config.items,
            &config.variables,
            &mut Vec::new(),
            false,
            &mut config_entries,
        );
        self.config_entries = config_entries;
        self.search_paths = config.settings.quick_launch.search_paths;
        self.refresh_dynamic(config, dynamic);
    }

    /// Azure DevOps の候補だけを組み直す。
    ///
    /// バックグラウンド同期の完了通知 (`WM_AZURE_DEVOPS_REFRESHED`) から使う。
    /// ここでフル `Index::build` をやり直すと、変わっていない apps /
    /// bookmarks / history まで道連れで再スキャンされる。特に
    /// `crate::apps::scan` はショートカット 1 件ごとに COM の ShellLink を
    /// 作るため実測で数十 ms かかり、UI スレッドを塞ぐ
    /// (`refresh_dynamic` が Recent/Frequent だけを差し替えるのと同じ理由)。
    pub fn refresh_azure(&mut self, config: &Config) {
        let settings = &config.settings.quick_launch;
        self.azure = azure_indexed(settings);
        self.azure_work_items = azure_work_item_entries(settings);
        self.azure_work_items_lower = super::search::LowerKeys::build_for(&self.azure_work_items);
    }

    /// 拡張から届いた全ブラウザのタブ一覧で、検索用候補を差し替える。
    /// タブの URL / タイトルは常駐プロセスのメモリだけに置き、設定や履歴には保存しない。
    pub(crate) fn set_browser_tabs(
        &mut self,
        tabs: &[(crate::browser_tabs::Browser, crate::browser_tabs::Tab)],
    ) {
        self.tabs = tabs
            .iter()
            .map(|(browser, tab)| Entry {
                name: if !tab.title.trim().is_empty() {
                    tab.title.clone()
                } else {
                    tab.url.clone()
                },
                breadcrumb: format!("{} — Current Tab", browser.label()),
                path: tab.url.clone(),
                action: Action::FocusBrowserTab(crate::browser_tabs::TabTarget {
                    browser: *browser,
                    tab_id: tab.id,
                    window_id: tab.window_id,
                }),
                branch: None,
            })
            .collect();
        self.tabs_lower = super::search::LowerKeys::build_for(&self.tabs);
    }
}

/// Azure DevOps の PR / Pipeline / Project 候補を、検索用の索引へ組む。
/// `Index::build` と `Index::refresh_azure` の共通部分。
fn azure_indexed(settings: &crate::config::QuickLaunchSettings) -> Vec<AzureIndexed> {
    let mut candidates = if settings.azure_devops.enabled {
        crate::azure_devops::project_candidates(&settings.azure_devops)
    } else {
        Vec::new()
    };
    candidates.extend(crate::azure_devops::cached_candidates(
        &settings.azure_devops,
    ));
    // 優先度を最優先しつつ、同一プロジェクト内では自分が関与する PR、
    // 日常的に開く Active PR、失敗した Pipeline の順に先頭へ置く。
    // 通常の使用履歴ランキングも後段で効く。
    candidates.sort_by_key(|candidate| (candidate.priority, azure_urgency(candidate)));
    candidates
        .into_iter()
        .map(|candidate| {
            let entry = azure_candidate_entry(candidate.clone());
            AzureIndexed {
                lower: super::search::LowerKeys::new(&entry),
                entry,
                kind: candidate.kind,
                status: candidate.status,
                is_mine: candidate.is_mine,
            }
        })
        .collect()
}

/// Work Item のキャッシュ候補。`azure_indexed` と同じく共通部分。
fn azure_work_item_entries(settings: &crate::config::QuickLaunchSettings) -> Vec<Entry> {
    crate::azure_devops::cached_work_item_candidates(&settings.azure_devops)
        .into_iter()
        .map(azure_candidate_entry)
        .collect()
}

/// config 由来の候補 (`config_entries`) に Recent/Frequent Folders を足して
/// 重複排除し、開いているウィンドウの一覧と合わせて返す。
/// `Index::build` と `Index::refresh_dynamic` の共通部分。
fn dynamic_entries(
    config_entries: &[Entry],
    settings: &crate::config::QuickLaunchSettings,
    dynamic: &Menus,
) -> (
    Vec<Entry>,
    Vec<super::search::LowerKeys>,
    Vec<Entry>,
    Vec<super::search::LowerKeys>,
) {
    let mut entries = config_entries.to_vec();
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

    let entries = dedup_by_path(entries);
    let entries_lower = super::search::LowerKeys::build_for(&entries);
    let windows_lower = super::search::LowerKeys::build_for(&windows);
    (entries, entries_lower, windows, windows_lower)
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

/// 自分が関与する PR、Active な PR、失敗した Pipeline の順に小さい値を返す。
/// `azure_candidates.sort_by_key` の第二キーとして使う。
fn azure_urgency(candidate: &crate::azure_devops::Candidate) -> u8 {
    match (&candidate.kind, candidate.status.as_str()) {
        (crate::azure_devops::Kind::PullRequest, _) if candidate.is_mine => 0,
        (crate::azure_devops::Kind::PullRequest, status)
            if status.eq_ignore_ascii_case("active") =>
        {
            1
        }
        (crate::azure_devops::Kind::Pipeline, status) if status.eq_ignore_ascii_case("failed") => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::azure_urgency;
    use crate::azure_devops::{Candidate, Kind};

    fn candidate(kind: Kind, status: &str, is_mine: bool) -> Candidate {
        Candidate {
            kind,
            status: status.to_string(),
            name: String::new(),
            detail: String::new(),
            url: String::new(),
            organization: String::new(),
            project: String::new(),
            aliases: Vec::new(),
            priority: 0,
            is_mine,
        }
    }

    #[test]
    fn own_pull_requests_rank_before_other_active_pull_requests() {
        let mine = candidate(Kind::PullRequest, "active", true);
        let others_active = candidate(Kind::PullRequest, "active", false);
        assert!(azure_urgency(&mine) < azure_urgency(&others_active));
    }

    #[test]
    fn own_completed_pull_request_still_ranks_before_active_ones_from_others() {
        let mine_completed = candidate(Kind::PullRequest, "completed", true);
        let others_active = candidate(Kind::PullRequest, "active", false);
        assert!(azure_urgency(&mine_completed) < azure_urgency(&others_active));
    }

    #[test]
    fn active_pull_requests_rank_before_failed_pipelines() {
        let active_pr = candidate(Kind::PullRequest, "active", false);
        let failed_pipeline = candidate(Kind::Pipeline, "failed", false);
        assert!(azure_urgency(&active_pr) < azure_urgency(&failed_pipeline));
    }
}
