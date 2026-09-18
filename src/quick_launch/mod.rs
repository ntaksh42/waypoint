//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use fuzzy_matcher::skim::SkimMatcherV2;

use crate::config::{Item, OpenMode};
use crate::quick_launch_history::Ranking;

mod azure;
mod azure_search;
mod index;
mod rank;
mod scoring;
mod search;
#[cfg(test)]
mod tests;

pub(crate) use azure::azure_suggest_entry;
pub use azure::{
    AzureCommand, AzureLiveRequest, PipelineFilter, PullRequestFilter, azure_command,
    azure_live_request,
};
pub(crate) use rank::search_entries;
pub(crate) use scoring::highlight_ranges;

use azure::AzureIndexed;

/// Skim の matcher は内部キャッシュを持つ。キー入力のたびに作り直さず共有する。
static FUZZY_MATCHER: std::sync::LazyLock<SkimMatcherV2> =
    std::sync::LazyLock::new(SkimMatcherV2::default);

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
/// Outlook メール検索モードに入るプレフィックス (末尾の半角スペース込み)。
pub const OUTLOOK_PREFIX: &str = "o ";
/// アプリ検索モードに入るプレフィックス (末尾の半角スペース込み)。
const APPS_PREFIX: &str = "a ";
/// 現在開いているブラウザタブを検索するプレフィックス (末尾の半角スペース込み)。
const TABS_PREFIX: &str = "t ";
/// フォルダをターミナルで開く検索モードに入るプレフィックス (末尾の半角スペース込み)。
const TERMINAL_PREFIX: &str = "ps ";
/// フォルダを設定済みエディターで開く検索モードに入るプレフィックス (末尾の半角スペース込み)。
const EDITOR_PREFIX: &str = "ed ";
/// Claude Code を表示名付きで起動するコマンドのプレフィックス。
pub const CLAUDE_CODE_PREFIX: &str = "cc ";
/// プロセス Kill 検索モードに入るプレフィックス (末尾の半角スペース込み、FR-9.15.2)。
pub const KILL_PROCESS_PREFIX: &str = "k ";
/// Web 検索モードに入るプレフィックス (FR-9.21)。
///
/// 他と違って末尾に半角スペースを含めない。`?? rust` と `??rust` の
/// どちらでも同じ検索語になるよう、残りを `trim` して扱うため
/// (PowerToys Run の `??` と同じ打ち心地に合わせる)。
pub const WEB_SEARCH_PREFIX: &str = "??";

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
    } else if query.starts_with(TABS_PREFIX) {
        Some("TABS")
    } else if query.starts_with(TERMINAL_PREFIX) {
        Some("TERMINAL")
    } else if query.starts_with(EDITOR_PREFIX) {
        Some("EDITOR")
    } else if query.starts_with(CLAUDE_CODE_PREFIX) {
        Some("CLAUDE CODE")
    } else if query.starts_with(EVERYTHING_PREFIX) {
        Some("FILES")
    } else if query.starts_with(OUTLOOK_PREFIX) {
        Some("OUTLOOK")
    } else if query.starts_with(KILL_PROCESS_PREFIX) {
        Some("KILL")
    } else if query.starts_with(WEB_SEARCH_PREFIX) {
        Some("WEB")
    } else {
        None
    }
}

/// クエリからモードプレフィックス (`b `/`w `/`az pr ` 等) を除いた、
/// 実際にスコアリングへ渡る検索語を返す。`Index::search` の分岐と対象を
/// 揃えるためのもので、ハイライト表示 (`scoring::highlight_ranges`) が
/// 一致判定と同じ語を見るために使う。
pub fn effective_search_term(query: &str) -> &str {
    for prefix in [
        BOOKMARK_PREFIX,
        HISTORY_PREFIX,
        WINDOW_PREFIX,
        APPS_PREFIX,
        TABS_PREFIX,
        TERMINAL_PREFIX,
        EDITOR_PREFIX,
        CLAUDE_CODE_PREFIX,
        KILL_PROCESS_PREFIX,
        OUTLOOK_PREFIX,
    ] {
        if let Some(rest) = query.strip_prefix(prefix) {
            return rest;
        }
    }
    if let Some((_, rest)) = azure_command(query) {
        return rest;
    }
    if let Some(rest) = query.strip_prefix(WEB_SEARCH_PREFIX) {
        return rest.trim();
    }
    query
}

/// waypoint 自身の操作を通常検索へ載せる固定候補 (FR-9.20)。
///
/// config や外部データに依存しない静的な一覧なので、`Index` の
/// 構築・更新経路には乗せず `azure_command_entries` と同じ形で
/// 静的に持つ。`LowerKeys` も一度だけ作って使い回す。
pub(crate) fn builtin_command_entries() -> (&'static [Entry], &'static [search::LowerKeys]) {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        vec![
            Entry {
                name: "Settings".to_string(),
                breadcrumb: "Open waypoint settings".to_string(),
                path: String::new(),
                action: Action::OpenSettings,
                branch: None,
            },
            Entry {
                name: "Help".to_string(),
                breadcrumb: "Open waypoint help".to_string(),
                path: String::new(),
                action: Action::OpenHelp,
                branch: None,
            },
        ]
    });
    static LOWER: std::sync::LazyLock<Vec<search::LowerKeys>> =
        std::sync::LazyLock::new(|| search::LowerKeys::build_for_names(&ENTRIES));
    (&ENTRIES, &LOWER)
}

/// `cc <folder> "<sessionname>"` を Claude Code 起動候補へ変換する。
///
/// フォルダ部分を引用しなくても空白を含められるよう、最後の ` "` を区切りとする。
/// パスの存在確認は起動時まで遅延し、入力経路では文字列の切り分けだけに留める。
pub(crate) fn claude_code_entry(query: &str) -> Option<Entry> {
    let rest = query.strip_prefix(CLAUDE_CODE_PREFIX)?;
    let (folder, session_name) = rest.rsplit_once(" \"")?;
    let folder = folder.trim();
    let session_name = session_name.strip_suffix('"')?;
    if folder.is_empty() || session_name.is_empty() || session_name.contains('"') {
        return None;
    }
    Some(Entry {
        name: format!("Claude Code — {session_name}"),
        breadcrumb: format!("Start in {folder}"),
        path: folder.to_string(),
        action: Action::OpenClaudeCode(session_name.to_string()),
        branch: None,
    })
}

/// 検索結果を選んだときに行うアクション。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// フォルダを開く (新規ウィンドウ / 既存ウィンドウの再利用)。
    OpenFolder(OpenMode),
    /// 既に開いているウィンドウにフォーカスを移す。
    FocusWindow(isize),
    /// Chrome / Edge の拡張へ、現在開いているタブの前面化を依頼する。
    FocusBrowserTab(crate::browser_tabs::TabTarget),
    /// 既定のブラウザで URL を開く。
    OpenUrl(String),
    /// Windows の既定ハンドラーでファイル / フォルダを開く (Everything 結果用)。
    /// `OpenFolder` と違い newWindow / reuse の区別を持たない。
    OpenWithDefaultHandler,
    /// スタートメニューのショートカットを起動する。
    LaunchApp,
    /// フォルダを Windows Terminal (PowerShell 7) でカレントディレクトリとして開く
    /// (`ps ` プレフィックス、FR-9.15.1)。newWindow / reuse の区別は持たない。
    OpenInTerminal,
    /// フォルダを設定済みエディターで開く (`ed ` プレフィックス)。
    OpenInEditor(String),
    /// Claude Code を指定フォルダと表示名で起動する (`cc ` コマンド)。
    OpenClaudeCode(String),
    /// 検索欄へコマンドを補完する。候補の選択時に外部操作は行わない。
    ReplaceQuery(String),
    /// `az wit` のローカルキャッシュ検索で見つからなかったとき、明示的な
    /// 選択をトリガーに Azure DevOps へライブ全文検索を投げる。
    AzureLiveWorkItemSearch(String),
    /// `az pr` 等のローカルキャッシュ検索で見つからなかったとき、明示的な
    /// 選択をトリガーに、打ち切り期間を広げて PR を再取得する
    /// (Azure DevOps の PR API に全文検索は無いため、広く取ってから
    /// ローカルでキーワードフィルタする)。
    AzureLivePullRequestSearch {
        filter: PullRequestFilter,
        query: String,
    },
    /// Pipeline は永続キャッシュを持たないので、`az pipeline ` に入るたびに
    /// 明示的な選択をトリガーに Live 検索を投げる
    /// (`AzureLiveWorkItemSearch` / `AzureLivePullRequestSearch` と同じ形)。
    AzureLivePipelineSearch {
        filter: PipelineFilter,
        query: String,
    },
    /// `az optimize`（`suggest` / `rank` でも入れる）— 直近アクティビティから
    /// 優先 Project / Iteration を自動更新する。
    AzureOptimize,
    /// waypoint 自身の設定画面 (`waypoint-settings.exe`) を開く (FR-9.20)。
    OpenSettings,
    /// waypoint の機能一覧ページ (`help\index.html`) を開く (FR-9.22)。
    OpenHelp,
    /// 設定した検索エンジンの検索 URL を既定ブラウザで開く
    /// (`??` プレフィックス、FR-9.21)。URL は `path` に組み立て済み。
    WebSearch,
    /// 選択したプロセスを確認なしで即時終了する (`k ` プレフィックス、FR-9.15.2)。
    KillProcess(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub breadcrumb: String,
    pub path: String,
    pub action: Action,
    /// showBranch が真の Folder 項目のみ。構築時にまとめて読む。
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
            Action::FocusWindow(_)
            | Action::FocusBrowserTab(_)
            | Action::OpenUrl(_)
            | Action::OpenInTerminal
            | Action::OpenInEditor(_)
            | Action::OpenClaudeCode(_)
            | Action::ReplaceQuery(_)
            | Action::AzureLiveWorkItemSearch(_)
            | Action::AzureLivePullRequestSearch { .. }
            | Action::AzureLivePipelineSearch { .. }
            | Action::AzureOptimize
            | Action::OpenSettings
            | Action::OpenHelp
            // 検索語は毎回異なりうるので永続化しない (FR-9.21)
            | Action::WebSearch
            // プロセスは永続化する対象ではない (PID は再起動のたびに変わる)
            | Action::KillProcess(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    /// config 由来の候補 (Recent/Frequent Folders を含まない)。
    /// `refresh_dynamic` が Recent/Frequent だけを組み直す際の元データ。
    pub(crate) config_entries: Vec<Entry>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) entries_lower: Vec<search::LowerKeys>,
    pub(crate) bookmarks: Vec<Entry>,
    pub(crate) bookmarks_lower: Vec<search::LowerKeys>,
    pub(crate) history: Vec<Entry>,
    pub(crate) history_lower: Vec<search::LowerKeys>,
    pub(crate) azure: Vec<AzureIndexed>,
    pub(crate) azure_work_items: Vec<Entry>,
    pub(crate) azure_work_items_lower: Vec<search::LowerKeys>,
    pub(crate) windows: Vec<Entry>,
    pub(crate) windows_lower: Vec<search::LowerKeys>,
    pub(crate) apps: Vec<Entry>,
    pub(crate) apps_lower: Vec<search::LowerKeys>,
    pub(crate) tabs: Vec<Entry>,
    pub(crate) tabs_lower: Vec<search::LowerKeys>,
    /// `entries` のうちフォルダ (`Action::OpenFolder`) だけを `OpenInTerminal`
    /// へ差し替えた索引 (`ps ` プレフィックス、FR-9.15.1)。新規データソースは
    /// 持たず、`entries` の更新のたびに組み直す。
    pub(crate) terminal_folders: Vec<Entry>,
    pub(crate) terminal_folders_lower: Vec<search::LowerKeys>,
    /// `entries` のフォルダを設定済みエディターで開く候補へ差し替えた索引。
    pub(crate) editor_folders: Vec<Entry>,
    pub(crate) editor_folders_lower: Vec<search::LowerKeys>,
    /// `entries` のうちフォルダだけを `cc ` 用の `ReplaceQuery` へ差し替えた索引
    /// (FR-9.15.4)。選択するとセッション名の入力へ進めるよう検索欄を補完する。
    pub(crate) claude_code_folders: Vec<Entry>,
    pub(crate) claude_code_folders_lower: Vec<search::LowerKeys>,
    pub(crate) search_paths: bool,
    /// Web 検索 (`??`、FR-9.21) で使うエンジン。無効化時は `None`。
    /// 候補は入力から組み立てる 1 件だけなので、索引は持たない
    /// (生成は `quick_launch_window::search`)。
    pub(crate) web_search: Option<crate::web_search::Engine>,
    pub(crate) ranking: Ranking,
}

/// 実行中プロセスのスナップショットから `k ` の検索候補を作る (FR-9.15.2)。
///
/// キー入力のたびに `crate::process::list_processes()` を都度呼んで組み立てる
/// ため、`Index` にはキャッシュを持たせない (kill 直後の一覧を古いままに
/// しないため)。呼び出し側 (`quick_launch_window::search`) が生成し、
/// 検索は通常の `search_entries` に乗せる。
pub fn kill_process_entries() -> Vec<Entry> {
    crate::process::list_processes()
        .into_iter()
        .map(|process| Entry {
            name: process.name,
            breadcrumb: format!("Kill Process — PID {}", process.pid),
            path: String::new(),
            action: Action::KillProcess(process.pid),
            branch: None,
        })
        .collect()
}

/// `??` の検索語から Web 検索の確定候補を 1 件作る (FR-9.21)。
///
/// 索引を引かず入力だけから組み立てるため、`Index` ではなく
/// 呼び出し側 (`quick_launch_window::search`) が所有する。
pub fn web_search_entry(engine: crate::web_search::Engine, query: &str) -> Entry {
    let query = query.trim();
    Entry {
        name: if query.is_empty() {
            engine.label().to_string()
        } else {
            query.to_string()
        },
        breadcrumb: format!("Search with {}", engine.label()),
        path: engine.url_for(query),
        action: Action::WebSearch,
        branch: None,
    }
}
