//! 検索・スコアリング。

use super::azure::{AzureCommand, azure_command, azure_command_entries};
use super::rank::{search_entries, search_entries_cached, search_entries_cached_multi};
use super::{
    APPS_PREFIX, AZURE_DEVOPS_PREFIX, BOOKMARK_PREFIX, CLAUDE_CODE_PREFIX, EDITOR_PREFIX, Entry,
    HISTORY_PREFIX, Index, TABS_PREFIX, TERMINAL_PREFIX, WINDOW_PREFIX,
};

/// `Entry::name` / `breadcrumb` / `path` の小文字化済みキャッシュ。
///
/// `Index::build` 時に候補 1 件につき 1 回だけ計算する。キー入力のたびに
/// 全候補分の `to_lowercase` を再アロケーションしていたのが検索の主要な
/// コストだったため (候補数千件規模で無視できない遅延になる)、`entries` /
/// `bookmarks` / `history` など件数が伸びやすい候補群にはこれを使う。
#[derive(Debug, Clone, Default)]
pub(crate) struct LowerKeys {
    name: String,
    breadcrumb: String,
    pub(super) path: String,
}

/// スコアリングが見る、小文字化済みのフィールド一式。
///
/// `path` は「マッチ対象に含めるか」(`search_paths`) を反映した `Option`、
/// `path_lower` は常に実体。使用履歴の順位付け (`Ranking::rank_lower`) が
/// パスの小文字化を要求するため、マッチ対象でなくても値自体は必要になる。
#[derive(Debug, Clone, Copy)]
pub(super) struct Fields<'a> {
    pub(super) name: &'a str,
    pub(super) breadcrumb: &'a str,
    pub(super) path: Option<&'a str>,
    pub(super) path_lower: &'a str,
}

impl<'a> Fields<'a> {
    /// 事前計算済みの `LowerKeys` から組み立てる。
    pub(super) fn from_keys(keys: &'a LowerKeys, search_paths: bool) -> Self {
        Self {
            name: &keys.name,
            breadcrumb: &keys.breadcrumb,
            path: search_paths.then_some(keys.path.as_str()),
            path_lower: &keys.path,
        }
    }
}

impl LowerKeys {
    pub(crate) fn new(entry: &Entry) -> Self {
        Self {
            name: entry.name.to_lowercase(),
            breadcrumb: entry.breadcrumb.to_lowercase(),
            path: entry.path.to_lowercase(),
        }
    }

    pub(crate) fn build_for(entries: &[Entry]) -> Vec<Self> {
        entries.iter().map(Self::new).collect()
    }

    /// 名前だけをマッチ対象にする検索キー。固定のコマンド候補 (FR-9.20)
    /// 向け。breadcrumb は説明文なので、そのまま検索対象にすると
    /// "Open waypoint settings" が `waypoint` や `open` のような普通の
    /// 検索語に tier4 / tier7 で一致し、無関係な検索へ割り込んでしまう
    /// (実測でフォルダ検索の結果に混ざった)。表示には使うのでキーだけ空にする。
    pub(crate) fn build_for_names(entries: &[Entry]) -> Vec<Self> {
        entries
            .iter()
            .map(|entry| Self {
                name: entry.name.to_lowercase(),
                breadcrumb: String::new(),
                path: String::new(),
            })
            .collect()
    }
}

impl Index {
    /// プレフィックス入力中は、対応する検索対象だけを検索する。
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        // `cc ` は入力文字列から確定候補を組み立てる。実体は
        // `quick_launch_window::search` が追加するため、索引は引かない。
        if query.starts_with(CLAUDE_CODE_PREFIX) {
            return Vec::new();
        }
        if let Some(rest) = query.strip_prefix(BOOKMARK_PREFIX) {
            return search_entries_cached(
                &self.bookmarks,
                &self.bookmarks_lower,
                rest,
                true,
                &self.ranking,
            );
        }
        if let Some(rest) = query.strip_prefix(HISTORY_PREFIX) {
            return search_entries_cached(
                &self.history,
                &self.history_lower,
                rest,
                true,
                &self.ranking,
            );
        }
        if let Some(rest) = query.strip_prefix(WINDOW_PREFIX) {
            return search_entries_cached(
                &self.windows,
                &self.windows_lower,
                rest,
                false,
                &self.ranking,
            );
        }
        if let Some(rest) = query.strip_prefix(APPS_PREFIX) {
            return search_entries_cached(&self.apps, &self.apps_lower, rest, false, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(TABS_PREFIX) {
            return search_entries_cached(&self.tabs, &self.tabs_lower, rest, true, &self.ranking);
        }
        if let Some(rest) = query.strip_prefix(TERMINAL_PREFIX) {
            return search_entries_cached(
                &self.terminal_folders,
                &self.terminal_folders_lower,
                rest,
                self.search_paths,
                &self.ranking,
            );
        }
        if let Some(rest) = query.strip_prefix(EDITOR_PREFIX) {
            return search_entries_cached(
                &self.editor_folders,
                &self.editor_folders_lower,
                rest,
                self.search_paths,
                &self.ranking,
            );
        }
        // `??` は索引を引かず入力から候補を組み立てるため、ここでは
        // 何も返さない (FR-9.21)。実体の生成は Azure のライブ検索と同じく
        // `quick_launch_window::search` が行う。
        if query.starts_with(super::WEB_SEARCH_PREFIX) {
            return Vec::new();
        }
        if query == AZURE_DEVOPS_PREFIX {
            return Vec::new();
        }
        if let Some(command_text) = super::azure::incomplete_azure_command(query) {
            let completions =
                search_entries(azure_command_entries(), command_text, false, &self.ranking);
            if !completions.is_empty() {
                return completions;
            }
        }
        if let Some((command, rest)) = azure_command(query) {
            return match command {
                AzureCommand::All => super::azure_search::search(
                    self.azure
                        .iter()
                        .map(|entry| (&entry.entry, &entry.lower))
                        .chain(
                            self.azure_work_items
                                .iter()
                                .zip(&self.azure_work_items_lower),
                        ),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::PullRequests(filter) => super::azure_search::search(
                    self.azure
                        .iter()
                        .filter(|entry| {
                            entry.kind == crate::azure_devops::Kind::PullRequest
                                && filter.status.matches(&entry.status)
                                && (!filter.mine || entry.is_mine)
                        })
                        .map(|entry| (&entry.entry, &entry.lower)),
                    rest,
                    true,
                    &self.ranking,
                ),
                AzureCommand::Projects => super::azure_search::search(
                    self.azure
                        .iter()
                        .filter(|entry| entry.kind == crate::azure_devops::Kind::Project)
                        .map(|entry| (&entry.entry, &entry.lower)),
                    rest,
                    true,
                    &self.ranking,
                ),
                // Pipeline は永続キャッシュを持たない (Live 検索専用) ので、
                // Work Item のライブ検索と同じく同期検索では何も返さない。
                // 実際の検索は `quick_launch_window::search` が
                // `search_pipelines_live_async` を叩いて行う。
                AzureCommand::Pipelines(_)
                | AzureCommand::WorkItems { .. }
                | AzureCommand::Suggest => Vec::new(),
            };
        }
        let (builtins, builtins_lower) = super::builtin_command_entries();
        search_entries_cached_multi(
            &[
                (&self.entries, &self.entries_lower),
                (&self.windows, &self.windows_lower),
                (&self.bookmarks, &self.bookmarks_lower),
                (&self.apps, &self.apps_lower),
                (builtins, builtins_lower),
            ],
            query,
            self.search_paths,
            &self.ranking,
        )
    }

    /// 絞り込みなし (空クエリ) のときに、Spotlight 風の区分見出し付き一覧を返す。
    /// 区分ごとに使用頻度順の上位 `SECTION_LIMIT` 件だけを載せ、一覧が
    /// 縦に伸びすぎないようにする。空の区分は含めない。
    pub fn sections(&self) -> Vec<(&'static str, Vec<&Entry>)> {
        const SECTION_LIMIT: usize = 6;
        [
            ("Folders", &self.entries, &self.entries_lower),
            ("Open Windows", &self.windows, &self.windows_lower),
            ("Bookmarks", &self.bookmarks, &self.bookmarks_lower),
            ("History", &self.history, &self.history_lower),
            ("Apps", &self.apps, &self.apps_lower),
        ]
        .into_iter()
        .filter_map(|(label, source, lower)| {
            let top = search_entries_cached(source, lower, "", self.search_paths, &self.ranking)
                .into_iter()
                .take(SECTION_LIMIT)
                .collect::<Vec<_>>();
            (!top.is_empty()).then_some((label, top))
        })
        .collect()
    }

    /// Work Item キャッシュは Index 構築時に読み込み済み。Quick Launch の
    /// キー入力経路では SQLite に触れず、ここで即時に候補の有無を判定する。
    /// `entries_lower` 等と同じく事前計算済みの `LowerKeys` を使う
    /// (毎キー入力で `to_lowercase` をやり直すと、事前キャッシュ化で
    /// 母集団が数百件規模に増えたときに体感できるカクつきになる。実測)。
    pub fn search_cached_work_items(&self, query: &str) -> Vec<&Entry> {
        super::azure_search::search(
            self.azure_work_items
                .iter()
                .zip(&self.azure_work_items_lower),
            query,
            true,
            &self.ranking,
        )
    }

    /// ライブ API の結果をメモリ上のキャッシュにも反映する。
    pub fn merge_cached_work_items(&mut self, entries: &[Entry]) {
        for entry in entries {
            if !self
                .azure_work_items
                .iter()
                .any(|cached| cached.path == entry.path)
            {
                self.azure_work_items_lower.push(LowerKeys::new(entry));
                self.azure_work_items.push(entry.clone());
            }
        }
    }
}
