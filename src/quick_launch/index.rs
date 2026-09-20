//! `Index::build` とその構築ヘルパー。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch_history::Ranking;

use super::azure::{AzureIndexed, azure_candidate_entry};
use super::rank::dedup_by_path;
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

        let (azure, azure_work_items) = azure_entries(settings);
        let azure_shortcuts = super::azure_shortcut_entries(&settings.azure_devops);

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
        let (terminal_folders, terminal_folders_lower) = terminal_folder_entries(&entries);
        let (editor_folders, editor_folders_lower) =
            editor_folder_entries(&entries, &settings.editor_command);
        let (claude_code_folders, claude_code_folders_lower) = claude_code_folder_entries(&entries);
        let (codex_folders, codex_folders_lower) = codex_folder_entries(&entries);

        Self {
            config_entries,
            entries,
            entries_lower,
            bookmarks,
            bookmarks_lower,
            history,
            history_lower,
            azure,
            azure_shortcuts,
            azure_work_items,
            azure_work_items_lower,
            windows,
            windows_lower,
            apps,
            apps_lower,
            tabs: Vec::new(),
            tabs_lower: Vec::new(),
            terminal_folders,
            terminal_folders_lower,
            editor_folders,
            editor_folders_lower,
            claude_code_folders,
            claude_code_folders_lower,
            codex_folders,
            codex_folders_lower,
            search_paths: settings.search_paths,
            web_search: settings
                .include_web_search
                .then_some(settings.web_search_engine),
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
        let (terminal_folders, terminal_folders_lower) = terminal_folder_entries(&entries);
        let (editor_folders, editor_folders_lower) =
            editor_folder_entries(&entries, &settings.editor_command);
        let (claude_code_folders, claude_code_folders_lower) = claude_code_folder_entries(&entries);
        let (codex_folders, codex_folders_lower) = codex_folder_entries(&entries);
        self.entries = entries;
        self.entries_lower = entries_lower;
        self.windows = windows;
        self.windows_lower = windows_lower;
        self.terminal_folders = terminal_folders;
        self.terminal_folders_lower = terminal_folders_lower;
        self.editor_folders = editor_folders;
        self.editor_folders_lower = editor_folders_lower;
        self.claude_code_folders = claude_code_folders;
        self.claude_code_folders_lower = claude_code_folders_lower;
        self.codex_folders = codex_folders;
        self.codex_folders_lower = codex_folders_lower;
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
        let settings = &config.settings.quick_launch;
        self.search_paths = settings.search_paths;
        self.web_search = settings
            .include_web_search
            .then_some(settings.web_search_engine);
        self.refresh_dynamic(config, dynamic);
    }

    /// 構築済みの Azure DevOps 候補だけを検索用の索引へ適用する。
    ///
    /// SQLite などの I/O は呼び出し元で済ませ、このメソッドはメモリ上の
    /// 候補を変換・交換するだけにする。
    pub(crate) fn refresh_azure_candidates(
        &mut self,
        settings: &crate::config::QuickLaunchSettings,
        groups: crate::azure_devops::CachedCandidateGroups,
    ) {
        (self.azure, self.azure_work_items) = azure_entries_from_candidates(settings, groups);
        self.azure_shortcuts = super::azure_shortcut_entries(&settings.azure_devops);
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
                    url: tab.url.clone(),
                }),
                branch: None,
            })
            .collect();
        self.tabs_lower = super::search::LowerKeys::build_for(&self.tabs);
    }
}

/// Azure DevOps キャッシュを読み、検索用候補へ変換する起動時経路。
fn azure_entries(settings: &crate::config::QuickLaunchSettings) -> (Vec<AzureIndexed>, Vec<Entry>) {
    let (pull_requests, work_items) =
        crate::azure_devops::cached_candidate_groups(&settings.azure_devops);
    azure_entries_from_candidates(
        settings,
        crate::azure_devops::CachedCandidateGroups {
            pull_requests,
            work_items,
        },
    )
}

/// メモリ上の Azure DevOps 候補を検索用の索引へ変換する。
fn azure_entries_from_candidates(
    settings: &crate::config::QuickLaunchSettings,
    groups: crate::azure_devops::CachedCandidateGroups,
) -> (Vec<AzureIndexed>, Vec<Entry>) {
    let mut candidates = if settings.azure_devops.enabled {
        crate::azure_devops::project_candidates(&settings.azure_devops)
    } else {
        Vec::new()
    };
    candidates.extend(groups.pull_requests);
    // 優先度を最優先しつつ、同一プロジェクト内では自分が関与する PR、
    // 日常的に開く Active PR、失敗した Pipeline の順に先頭へ置く。
    // 通常の使用履歴ランキングも後段で効く。
    candidates.sort_by_key(|candidate| (candidate.priority, azure_urgency(candidate)));
    let indexed = candidates
        .into_iter()
        .map(|candidate| {
            let entry = azure_candidate_entry(candidate.clone());
            AzureIndexed {
                lower: super::search::LowerKeys::new(&entry),
                entry,
                kind: candidate.kind,
                status: candidate.status,
                is_mine: candidate.is_mine,
                is_author: candidate.is_author,
                is_reviewer: candidate.is_reviewer,
                needs_my_review: candidate.needs_my_review,
                waiting_for_others: candidate.waiting_for_others,
                is_draft: candidate.is_draft,
                ready_to_complete: candidate.ready_to_complete,
                is_stale: candidate.is_stale,
            }
        })
        .collect();
    let work_items = groups
        .work_items
        .into_iter()
        .map(azure_candidate_entry)
        .collect();
    (indexed, work_items)
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

/// `entries` のうちフォルダだけを `Action::OpenInTerminal` に差し替えた索引を作る
/// (`ps ` プレフィックス、FR-9.15.1)。newWindow/reuse の区別は持たないため、
/// 元の `OpenMode` は捨てる。
fn terminal_folder_entries(entries: &[Entry]) -> (Vec<Entry>, Vec<super::search::LowerKeys>) {
    let terminal_folders: Vec<Entry> = entries
        .iter()
        .filter(|entry| matches!(entry.action, Action::OpenFolder(_)))
        .map(|entry| Entry {
            name: entry.name.clone(),
            breadcrumb: entry.breadcrumb.clone(),
            path: entry.path.clone(),
            action: Action::OpenInTerminal,
            branch: entry.branch.clone(),
        })
        .collect();
    let terminal_folders_lower = super::search::LowerKeys::build_for(&terminal_folders);
    (terminal_folders, terminal_folders_lower)
}

/// `entries` のうちフォルダだけを `Action::OpenInEditor` に差し替えた索引を作る。
/// 新規データソースは持たず、`ed ` プレフィックスでの絞り込みだけに使う。
fn editor_folder_entries(
    entries: &[Entry],
    editor_command: &str,
) -> (Vec<Entry>, Vec<super::search::LowerKeys>) {
    let editor_folders: Vec<Entry> = entries
        .iter()
        .filter(|entry| matches!(entry.action, Action::OpenFolder(_)))
        .map(|entry| Entry {
            name: entry.name.clone(),
            breadcrumb: entry.breadcrumb.clone(),
            path: entry.path.clone(),
            action: Action::OpenInEditor(editor_command.to_string()),
            branch: entry.branch.clone(),
        })
        .collect();
    let editor_folders_lower = super::search::LowerKeys::build_for(&editor_folders);
    (editor_folders, editor_folders_lower)
}

/// `entries` のうちフォルダだけを `cc ` 用の `Action::ReplaceQuery` に
/// 差し替えた索引を作る (FR-9.15.4)。選択すると検索欄が
/// `cc <folder> ` へ置き換わり、続けてセッション名を入力させる。
fn claude_code_folder_entries(entries: &[Entry]) -> (Vec<Entry>, Vec<super::search::LowerKeys>) {
    let claude_code_folders: Vec<Entry> = entries
        .iter()
        .filter(|entry| matches!(entry.action, Action::OpenFolder(_)))
        .map(|entry| Entry {
            name: entry.name.clone(),
            breadcrumb: entry.breadcrumb.clone(),
            path: entry.path.clone(),
            action: Action::ReplaceQuery(format!("{}{} ", super::CLAUDE_CODE_PREFIX, entry.path)),
            branch: entry.branch.clone(),
        })
        .collect();
    let claude_code_folders_lower = super::search::LowerKeys::build_for(&claude_code_folders);
    (claude_code_folders, claude_code_folders_lower)
}

/// `entries` のうちフォルダだけを `Action::OpenCodex` に差し替えた索引を作る
/// (FR-9.15.5)。Codex CLI には表示名の指定が無いため、`cc ` と違って
/// 検索欄の補完を挟まず、選択で即起動する。
fn codex_folder_entries(entries: &[Entry]) -> (Vec<Entry>, Vec<super::search::LowerKeys>) {
    let codex_folders: Vec<Entry> = entries
        .iter()
        .filter(|entry| matches!(entry.action, Action::OpenFolder(_)))
        .map(|entry| Entry {
            action: Action::OpenCodex,
            ..entry.clone()
        })
        .collect();
    let codex_folders_lower = super::search::LowerKeys::build_for(&codex_folders);
    (codex_folders, codex_folders_lower)
}

/// `inherited_show_branch` は祖先 Submenu の showBranch が真だったか。
/// 真なら配下の Folder は自身の showBranch を問わずブランチ名を持たせる
/// 設定した `showBranch` の継承規則に従う。
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
                    // ブランチ名の付与は構築時に済ませる。表示経路では読まない。
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
            branch: None,
            url: String::new(),
            organization: String::new(),
            project: String::new(),
            aliases: Vec::new(),
            priority: 0,
            is_mine,
            is_author: false,
            is_reviewer: false,
            needs_my_review: false,
            waiting_for_others: false,
            is_draft: false,
            ready_to_complete: false,
            is_stale: false,
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
