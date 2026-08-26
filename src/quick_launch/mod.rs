//! Quick Launch の検索インデックスと順位付け。
//!
//! Win32 の表示部分とは分離し、キー入力中はこのメモリ上のデータだけを検索する。

use fuzzy_matcher::skim::SkimMatcherV2;

use crate::config::{Item, OpenMode};
use crate::quick_launch_history::Ranking;

mod azure;
mod index;
mod scoring;
mod search;
#[cfg(test)]
mod tests;

pub(crate) use azure::azure_suggest_entry;
pub use azure::{AzureCommand, PipelineFilter, PullRequestFilter, azure_command};
pub(crate) use search::search_entries;

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
/// アプリ検索モードに入るプレフィックス (末尾の半角スペース込み)。
const APPS_PREFIX: &str = "a ";
/// 現在開いているブラウザタブを検索するプレフィックス (末尾の半角スペース込み)。
const TABS_PREFIX: &str = "t ";

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
    /// Chrome / Edge の拡張へ、現在開いているタブの前面化を依頼する。
    FocusBrowserTab(crate::browser_tabs::TabTarget),
    /// 既定のブラウザで URL を開く。
    OpenUrl(String),
    /// Windows の既定ハンドラーでファイル / フォルダを開く (Everything 結果用)。
    /// `OpenFolder` と違い newWindow / reuse の区別を持たない。
    OpenWithDefaultHandler,
    /// スタートメニューのショートカットを起動する。
    LaunchApp,
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
    /// `az optimize`（`suggest` / `rank` でも入れる）— 直近のアサイン・
    /// メンションから優先 Project / Area を提案する専用画面を開く
    /// (設定エディタープロセスを別引数で起動する)。
    AzureSuggestPriorities,
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
            Action::FocusWindow(_)
            | Action::FocusBrowserTab(_)
            | Action::OpenUrl(_)
            | Action::ReplaceQuery(_)
            | Action::AzureLiveWorkItemSearch(_)
            | Action::AzureLivePullRequestSearch { .. }
            | Action::AzureSuggestPriorities => None,
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
    pub(crate) search_paths: bool,
    pub(crate) ranking: Ranking,
}
