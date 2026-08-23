//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch_history::Ranking;

/// ブックマーク検索モードに入るプレフィックス (末尾の半角スペース込み)。
const BOOKMARK_PREFIX: &str = "b ";
/// ブラウザ履歴検索モードに入るプレフィックス (末尾の半角スペース込み)。
const HISTORY_PREFIX: &str = "h ";
/// Azure DevOps 検索モードに入るプレフィックス (末尾の半角スペース込み)。
pub const AZURE_DEVOPS_PREFIX: &str = "az ";
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
    } else if query.starts_with(HISTORY_PREFIX) {
        Some("HISTORY")
    } else if query.starts_with(AZURE_DEVOPS_PREFIX) {
        Some("AZURE DEVOPS")
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

impl Entry {
    /// config へお気に入り登録できる候補なら `Item` へ変換する。
    ///
    /// ウィンドウ (`FocusWindow`) と URL (`OpenUrl`、`b ` で既に検索できる)
    /// は永続化する対象ではないので None を返す。`OpenWithDefaultHandler`
    /// は Everything の結果 (ファイル / フォルダ両方あり得る) なので、
    /// 実際のパスを見てどちらか判定する。
    pub fn to_item(&self) -> Option<Item> {
        match &self.action {
            Action::OpenFolder(open) => Some(Item::Folder {
                name: self.name.clone(),
                path: self.path.clone(),
                open: Some(*open),
                icon: None,
                show_branch: false,
            }),
            Action::OpenWithDefaultHandler if std::path::Path::new(&self.path).is_dir() => {
                Some(Item::Folder {
                    name: self.name.clone(),
                    path: self.path.clone(),
                    open: None,
                    icon: None,
                    show_branch: false,
                })
            }
            Action::OpenWithDefaultHandler | Action::LaunchApp => Some(Item::File {
                name: self.name.clone(),
                path: self.path.clone(),
                icon: None,
            }),
            Action::FocusWindow(_) | Action::OpenUrl(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: Vec<Entry>,
    bookmarks: Vec<Entry>,
    history: Vec<Entry>,
    azure: Vec<AzureIndexed>,
    windows: Vec<Entry>,
    apps: Vec<Entry>,
    search_paths: bool,
    ranking: Ranking,
}

#[derive(Debug, Clone)]
struct AzureIndexed {
    entry: Entry,
    kind: crate::azure_devops::Kind,
    status: String,
    is_mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCommand {
    All,
    PullRequests(PullRequestFilter),
    Pipelines(PipelineFilter),
    Projects,
    WorkItems,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullRequestFilter {
    status: crate::azure_devops::PullRequestStatus,
    mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFilter {
    All,
    Definitions,
    Failed,
}

/// `az` のサブコマンドを分解する。未知の先頭語は検索語として扱うので、
/// `az waypoint` は横断検索、`az pr waypoint` は PR 検索になる。
pub fn azure_command(query: &str) -> Option<(AzureCommand, &str)> {
    let rest = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    let (first, remaining) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(first, remaining)| {
            (first, remaining.trim_start())
        });
    let command = match first.to_ascii_lowercase().as_str() {
        "pr" | "prs" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::All,
            mine: false,
        })),
        "pr-a" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Active,
            mine: false,
        })),
        "pr-c" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Completed,
            mine: false,
        })),
        "pr-ab" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Abandoned,
            mine: false,
        })),
        "pr-mine" | "pr-me" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::All,
            mine: true,
        })),
        "pr-a-mine" | "pr-a-me" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Active,
            mine: true,
        })),
        "pipeline" | "pipelines" | "pipe" | "build" | "builds" => {
            Some(AzureCommand::Pipelines(PipelineFilter::All))
        }
        "pipeline-def" | "pipeline-definition" | "pipeline-definitions" => {
            Some(AzureCommand::Pipelines(PipelineFilter::Definitions))
        }
        "pipeline-failed" | "pipeline-fail" | "build-failed" => {
            Some(AzureCommand::Pipelines(PipelineFilter::Failed))
        }
        "project" | "projects" => Some(AzureCommand::Projects),
        "wit" | "wi" | "workitem" | "workitems" => Some(AzureCommand::WorkItems),
        _ => None,
    };
    Some(command.map_or((AzureCommand::All, rest), |command| (command, remaining)))
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
                entry: Entry {
                    name: candidate.name,
                    breadcrumb: if candidate.aliases.is_empty() {
                        candidate.detail
                    } else {
                        format!("{} — {}", candidate.detail, candidate.aliases.join(" "))
                    },
                    path: candidate.url.clone(),
                    action: Action::OpenUrl(candidate.url),
                    branch: None,
                },
                kind: candidate.kind,
                status: candidate.status,
                is_mine: candidate.is_mine,
            })
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
            windows,
            apps,
            search_paths: settings.search_paths,
            ranking: Ranking::load(),
        }
    }

    /// プレフィックス入力中は、対応する検索対象だけを検索する。
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        if let Some(rest) = query.strip_prefix(BOOKMARK_PREFIX) {
            return search_entries(&self.bookmarks, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(HISTORY_PREFIX) {
            return search_entries(&self.history, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(WINDOW_PREFIX) {
            return search_entries(&self.windows, rest, false, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(APPS_PREFIX) {
            return search_entries(&self.apps, rest, false, &self.ranking);
        }
        if let Some((command, rest)) = azure_command(query) {
            return match command {
                AzureCommand::All => search_entries(
                    self.azure.iter().map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::PullRequests(filter) => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| {
                            entry.kind == crate::azure_devops::Kind::PullRequest
                                && filter.status.matches(&entry.status)
                                && (!filter.mine || entry.is_mine)
                        })
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::Pipelines(filter) => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| {
                            entry.kind == crate::azure_devops::Kind::Pipeline
                                && match filter {
                                    PipelineFilter::All => true,
                                    PipelineFilter::Definitions => {
                                        entry.status.eq_ignore_ascii_case("definition")
                                    }
                                    PipelineFilter::Failed => {
                                        entry.status.eq_ignore_ascii_case("failed")
                                    }
                                }
                        })
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::Projects => search_entries(
                    self.azure
                        .iter()
                        .filter(|entry| entry.kind == crate::azure_devops::Kind::Project)
                        .map(|entry| &entry.entry),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::WorkItems => Vec::new(),
            };
        }
        search_entries(&self.entries, query, self.search_paths, &self.ranking)
    }
}

fn search_entries<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    query: &str,
    search_paths: bool,
    ranking: &Ranking,
) -> Vec<&'a Entry> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matches: Vec<(u8, (u64, u64), usize, &Entry)> = entries
        .into_iter()
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
            history: vec![Entry {
                name: "WayPoint pull request".into(),
                breadcrumb: "Chrome History".into(),
                path: "https://github.com/example/waypoint/pull/1".into(),
                action: Action::OpenUrl("https://github.com/example/waypoint/pull/1".into()),
                branch: None,
            }],
            azure: vec![AzureIndexed {
                entry: Entry {
                    name: "PR 42: Add Azure search".into(),
                    breadcrumb: "Azure DevOps — org/Waypoint — active — wp".into(),
                    path: "https://dev.azure.com/org/Waypoint/_git/app/pullrequest/42".into(),
                    action: Action::OpenUrl(
                        "https://dev.azure.com/org/Waypoint/_git/app/pullrequest/42".into(),
                    ),
                    branch: None,
                },
                kind: crate::azure_devops::Kind::PullRequest,
                status: "active".into(),
                is_mine: true,
            }],
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

    /// タイトルにアプリ名が出ないウィンドウも、所有プロセス名で
    /// 見つけられる (`w chrome` のような検索)。
    #[test]
    fn window_search_also_matches_the_owning_process_name() {
        use crate::dynamic::WindowEntry;

        let dynamic = Menus {
            all_windows: vec![WindowEntry {
                title: "新しいタブ".to_string(),
                hwnd: 999,
                process_name: "chrome.exe".to_string(),
            }],
            ..Menus::default()
        };
        let index = Index::build(&Config::default(), &dynamic);
        let found = index.search("w chrome");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, Action::FocusWindow(999));
    }

    /// トレイの "Current Windows" 表示は件数を絞るが (ITEM_LIMIT) 、
    /// Quick Launch の検索索引は絞られていない全件から作られること。
    /// 絞られた current_windows だけを索引に使うと、開いているウィンドウが
    /// 多い環境で一部が `w ` 検索に一切ヒットしなくなる (実際の不具合)。
    #[test]
    fn window_search_uses_all_windows_not_the_truncated_tray_list() {
        use crate::dynamic::WindowEntry;

        let window = |hwnd: isize| WindowEntry {
            title: format!("Window {hwnd}"),
            hwnd,
            process_name: "app.exe".to_string(),
        };

        let dynamic = Menus {
            // トレイ表示用は 1 件だけに絞られているとする
            current_windows: vec![window(1)],
            // 検索索引用は絞られていない全件
            all_windows: vec![window(1), window(2), window(3)],
            ..Menus::default()
        };
        let index = Index::build(&Config::default(), &dynamic);
        let found = index.search("w window");
        assert_eq!(found.len(), 3);
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
    fn history_prefix_switches_to_history_only_search() {
        let index = index();
        let found = index.search("h github.com/example");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "WayPoint pull request");
    }

    #[test]
    fn without_the_history_prefix_history_is_not_searched() {
        let index = index();
        assert!(index.search("pull request").is_empty());
    }

    #[test]
    fn azure_pr_status_command_filters_cached_pull_requests() {
        let index = index();
        assert_eq!(index.search("az pr-a azure").len(), 1);
        assert_eq!(index.search("az pr-a-mine azure").len(), 1);
        assert!(index.search("az pr-c azure").is_empty());
        assert_eq!(index.search("az wp").len(), 1);
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
        assert_eq!(prefix_badge("h waypoint"), Some("HISTORY"));
        assert_eq!(prefix_badge("az pr-a waypoint"), Some("AZURE DEVOPS"));
        assert_eq!(prefix_badge("w notepad"), Some("WINDOWS"));
        assert_eq!(prefix_badge("a code"), Some("APPS"));
        assert_eq!(prefix_badge("f cargo.toml"), Some("FILES"));
        assert_eq!(prefix_badge("plain query"), None);
        assert_eq!(prefix_badge(""), None);
    }

    #[test]
    fn azure_command_recognizes_all_supported_subcommands() {
        assert_eq!(
            azure_command("az pr-c done"),
            Some((
                AzureCommand::PullRequests(PullRequestFilter {
                    status: crate::azure_devops::PullRequestStatus::Completed,
                    mine: false,
                }),
                "done"
            ))
        );
        assert_eq!(
            azure_command("az wit bug"),
            Some((AzureCommand::WorkItems, "bug"))
        );
        assert_eq!(
            azure_command("az pipelines release"),
            Some((AzureCommand::Pipelines(PipelineFilter::All), "release"))
        );
        assert_eq!(
            azure_command("az pipeline-failed release"),
            Some((AzureCommand::Pipelines(PipelineFilter::Failed), "release"))
        );
        assert_eq!(
            azure_command("az pr-a-mine launcher"),
            Some((
                AzureCommand::PullRequests(PullRequestFilter {
                    status: crate::azure_devops::PullRequestStatus::Active,
                    mine: true,
                }),
                "launcher"
            ))
        );
        assert_eq!(
            azure_command("az workitems defect"),
            Some((AzureCommand::WorkItems, "defect"))
        );
        assert_eq!(
            azure_command("az platform"),
            Some((AzureCommand::All, "platform"))
        );
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

    #[test]
    fn open_folder_entry_converts_to_folder_item_with_same_open_mode() {
        let entry = Entry {
            name: "Docs".into(),
            breadcrumb: String::new(),
            path: r"E:\waypoint\docs".into(),
            action: Action::OpenFolder(OpenMode::Reuse),
            branch: None,
        };
        let item = entry.to_item().expect("folder entries are addable");
        assert_eq!(
            item,
            Item::Folder {
                name: "Docs".into(),
                path: r"E:\waypoint\docs".into(),
                open: Some(OpenMode::Reuse),
                icon: None,
                show_branch: false,
            }
        );
    }

    /// FocusWindow / OpenUrl は config へ永続化できないので、
    /// お気に入り登録の対象外として None を返す。
    #[test]
    fn window_and_url_entries_are_not_addable() {
        let window = Entry {
            name: "Notepad".into(),
            breadcrumb: String::new(),
            path: String::new(),
            action: Action::FocusWindow(1),
            branch: None,
        };
        let url = Entry {
            name: "Example".into(),
            breadcrumb: String::new(),
            path: "https://example.com/".into(),
            action: Action::OpenUrl("https://example.com/".into()),
            branch: None,
        };
        assert!(window.to_item().is_none());
        assert!(url.to_item().is_none());
    }

    /// Everything の結果 (`OpenWithDefaultHandler`) は実際のパスを見て
    /// フォルダかファイルかを判定する。このリポジトリ自身を使って確認する。
    #[test]
    fn everything_result_becomes_folder_item_when_path_is_a_directory() {
        let entry = Entry {
            name: "src".into(),
            breadcrumb: String::new(),
            path: format!("{}\\src", env!("CARGO_MANIFEST_DIR")),
            action: Action::OpenWithDefaultHandler,
            branch: None,
        };
        let item = entry.to_item().expect("existing directories are addable");
        assert!(matches!(item, Item::Folder { .. }));
    }

    #[test]
    fn everything_result_becomes_file_item_when_path_is_not_a_directory() {
        let entry = Entry {
            name: "Cargo.toml".into(),
            breadcrumb: String::new(),
            path: format!("{}\\Cargo.toml", env!("CARGO_MANIFEST_DIR")),
            action: Action::OpenWithDefaultHandler,
            branch: None,
        };
        let item = entry.to_item().expect("existing files are addable");
        assert!(matches!(item, Item::File { .. }));
    }

    #[test]
    fn launch_app_entry_converts_to_file_item() {
        let entry = Entry {
            name: "Visual Studio Code".into(),
            breadcrumb: String::new(),
            path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
            action: Action::LaunchApp,
            branch: None,
        };
        let item = entry.to_item().expect("apps are addable");
        assert_eq!(
            item,
            Item::File {
                name: "Visual Studio Code".into(),
                path: r"C:\Start Menu\Visual Studio Code.lnk".into(),
                icon: None,
            }
        );
    }
}
