//! Azure DevOps の Live 検索 (`az `、FR-9.18)。
//!
//! `search.rs` から分離した。キャッシュ検索と違い API を非同期に叩き、
//! 応答を `WM_COPYDATA` 相当のメッセージで受けて一覧へ流し込むまでが
//! 一続きの関心事なので、検索窓そのものの処理とは別ファイルにする。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::HSTRING;

use super::search::{build_rows, next_azure_reply_id, populate_empty_message, populate_list};
use super::{MAX_LIST_RESULTS, RowKind, STATE, State, WM_QUICK_LAUNCH_AZURE_RESULTS};
use crate::quick_launch::Entry;

const LIVE_SEARCH_COOLDOWN: Duration = Duration::from_secs(2);

/// `Ctrl+Enter` による Azure Live 検索の開始結果。
///
/// Azure コマンドではあるが、既存検索の実行中・クールダウン中などで開始を
/// 抑止した場合は通常の候補実行へフォールバックしてはいけない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AzureLiveSearchStart {
    Started,
    Suppressed,
    NotApplicable,
}

/// Live 検索の種別。`az <query>` (サブコマンド無し) は 3 種を同時に投げる
/// ため、実行中フラグとクールダウンを種別ごとに独立させる。1 本の gate を
/// 共有すると、同時に投げた 2 本目以降が自分自身に弾かれる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveSearchKind {
    WorkItems,
    PullRequests,
    Pipelines,
}

impl LiveSearchKind {
    fn slot(self) -> usize {
        match self {
            LiveSearchKind::WorkItems => 0,
            LiveSearchKind::PullRequests => 1,
            LiveSearchKind::Pipelines => 2,
        }
    }
}

#[derive(Default)]
struct LiveSearchSlot {
    active_key: Option<String>,
    last_finished: Option<(String, Instant)>,
}

#[derive(Default)]
pub(super) struct LiveSearchGate {
    slots: [LiveSearchSlot; 3],
}

impl LiveSearchGate {
    pub(super) fn try_start(&mut self, kind: LiveSearchKind, key: &str, now: Instant) -> bool {
        let key = key
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let slot = &mut self.slots[kind.slot()];
        if slot.active_key.is_some()
            || slot
                .last_finished
                .as_ref()
                .is_some_and(|(previous, finished)| {
                    previous == &key && now.duration_since(*finished) < LIVE_SEARCH_COOLDOWN
                })
        {
            return false;
        }
        slot.active_key = Some(key);
        true
    }

    pub(super) fn finish(&mut self, kind: LiveSearchKind, now: Instant) {
        let slot = &mut self.slots[kind.slot()];
        if let Some(key) = slot.active_key.take() {
            slot.last_finished = Some((key, now));
        }
    }
}

/// `az <query>` (サブコマンド無し) の `Ctrl+Enter` で 3 種の Live 検索を
/// 同時に投げたときの集約先。到着した種別から順に結果を積み、まだ応答が
/// 残っている間も届いた分だけ表示する (全種そろうまで空白にしない)。
#[derive(Default)]
pub(super) struct CombinedLiveSearch {
    pending: usize,
    work_items: Vec<Entry>,
    pull_requests: Vec<Entry>,
    pipelines: Vec<Entry>,
    /// 1 種でも結果を返したか。全種 0 件のときだけ「見つからない」を出す。
    messages: Vec<String>,
}

impl CombinedLiveSearch {
    pub(super) fn new(pending: usize) -> Self {
        Self {
            pending,
            ..Self::default()
        }
    }

    /// 1 種別の応答を取り込む。戻り値は表示に使う (候補一覧, 説明文)。
    pub(super) fn absorb(
        &mut self,
        kind: LiveSearchKind,
        candidates: Vec<Entry>,
        message: Option<String>,
    ) -> (Vec<Entry>, Option<String>) {
        match kind {
            LiveSearchKind::WorkItems => self.work_items = candidates,
            LiveSearchKind::PullRequests => self.pull_requests = candidates,
            LiveSearchKind::Pipelines => self.pipelines = candidates,
        }
        if let Some(message) = message {
            self.messages.push(message);
        }
        self.pending = self.pending.saturating_sub(1);
        let merged: Vec<Entry> = self
            .pull_requests
            .iter()
            .chain(self.work_items.iter())
            .chain(self.pipelines.iter())
            .take(MAX_LIST_RESULTS)
            .cloned()
            .collect();
        let message = if !merged.is_empty() {
            None
        } else if self.pending > 0 {
            Some("Searching Azure DevOps…".to_string())
        } else if self.messages.is_empty() {
            Some("No Azure DevOps results.".to_string())
        } else {
            Some(self.messages.join(" / "))
        };
        (merged, message)
    }
}


/// `az wit ` に入った。SQLite キャッシュ (バックグラウンド定期同期が
/// 埋める「最近更新された Work Item」) だけをローカル検索する。API は
/// 自動では呼ばない — 検索語があってキャッシュに無ければ、末尾に
/// 明示的なライブ検索の選択肢を 1 件足すだけに留める (ユーザー操作が
/// トリガー、入力のたびに Azure DevOps を叫ばない)。
pub(super) fn start_azure_work_item_query(state: &RefCell<State>, text: &str) {
    let (list, has_results) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        state.previous_query = None;
        state.highlight_term.clear();
        state.results = state
            .index
            .search_cached_work_items(text)
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .cloned()
            .collect();
        let trimmed = text.trim();
        if state.results.is_empty() && !trimmed.is_empty() {
            state.results.push(live_work_item_search_entry(trimmed));
        }
        state.empty_message = if state.results.is_empty() {
            Some(if trimmed.is_empty() {
                "No recently updated work items.".to_string()
            } else {
                "No matching work items.".to_string()
            })
        } else {
            None
        };
        (state.list, !state.results.is_empty())
    };
    let Some(list) = list else {
        return;
    };
    let (labels, rows, message) = {
        let mut state = state.borrow_mut();
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = if rows.is_empty() {
            vec![RowKind::Message]
        } else {
            rows.clone()
        };
        (labels, rows, state.empty_message.clone())
    };
    if has_results {
        populate_list(list, &labels, &rows);
    } else {
        populate_empty_message(list, message.as_deref());
    }
}

/// `az suggest` に入った。検索対象を持たないコマンドなので、確定候補を
/// 1 件だけ表示する (Enter または選択で `AzureOptimize` が走る)。
pub(super) fn show_azure_suggest_entry(state: &RefCell<State>) {
    let (list, labels, rows) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        state.azure_work_items_active = false;
        state.previous_query = None;
        state.highlight_term.clear();
        state.empty_message = None;
        state.results = vec![crate::quick_launch::azure_suggest_entry()];
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = rows.clone();
        (state.list, labels, rows)
    };
    let Some(list) = list else {
        return;
    };
    populate_list(list, &labels, &rows);
}

/// キャッシュ検索が 0 件だったときにリストへ足す、ライブ検索への入口。
fn live_work_item_search_entry(query: &str) -> Entry {
    Entry {
        name: format!("Search Azure DevOps for \"{query}\""),
        breadcrumb: "Not in cache — press Enter to search live".to_string(),
        path: String::new(),
        action: crate::quick_launch::Action::AzureLiveWorkItemSearch(query.to_string()),
        branch: None,
    }
}

/// `AzureLiveWorkItemSearch` が選ばれた。ウィンドウは閉じずにその場で
/// API 検索を投げ、結果が届いたらリストだけ差し替える。
pub(super) fn start_azure_work_item_live_search(state: &RefCell<State>, query: &str) -> bool {
    let key = format!("wit:{}", query.trim());
    if !state.borrow_mut().azure_live_search_gate.try_start(
        LiveSearchKind::WorkItems,
        &key,
        Instant::now(),
    ) {
        return false;
    }
    let (window, reply_id, settings) = {
        let mut state = state.borrow_mut();
        // 集約モード (`az <query>`) では 3 種を同時に投げるので、他種別の
        // 実行中フラグや到着済みの結果を潰してはいけない
        if state.azure_live_combined.is_none() {
            invalidate_azure_live_searches(&mut state);
            state.results.clear();
        }
        state.azure_work_items_active = true;
        state.azure_work_item_reply_id = next_azure_reply_id(state.azure_work_item_reply_id);
        state.azure_work_item_query = query.trim().to_string();
        state.highlight_term.clear();
        // 集約モードでは 3 種の進捗を 1 行にまとめる (種別ごとの文言で
        // 上書きし合うと、どの検索が走っているか読めなくなる)
        state.empty_message = Some(if state.azure_live_combined.is_some() {
            "Searching Azure DevOps…".to_string()
        } else {
            "Searching Azure DevOps work items…".to_string()
        });
        state.rows = vec![RowKind::Message];
        (
            state.window,
            state.azure_work_item_reply_id,
            state.azure_devops.clone(),
        )
    };
    let Some(list) = STATE.with(|state| state.borrow().list) else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::WorkItems, Instant::now());
        return false;
    };
    let progress = STATE.with(|state| state.borrow().empty_message.clone());
    populate_empty_message(list, progress.as_deref());
    let Some(window) = window else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::WorkItems, Instant::now());
        return false;
    };
    if !settings.enabled {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::WorkItems, Instant::now());
        set_azure_empty_message("Azure DevOps search is disabled in Settings.");
        return false;
    }
    crate::azure_devops::search_work_items_async(
        settings,
        query.trim().to_string(),
        reply_id,
        window,
        WM_QUICK_LAUNCH_AZURE_RESULTS,
    );
    true
}

/// Live 検索の応答を `State` へ反映する。集約モード (`az <query>`) では
/// 種別ごとの結果をマージし、単独種別なら従来どおり丸ごと置き換える。
/// 戻り値は借用解放後にリストへ流し込むための (labels, rows, message)。
fn apply_live_results(
    state: &mut State,
    kind: LiveSearchKind,
    candidates: Vec<Entry>,
    message: Option<String>,
) -> (Vec<HSTRING>, Vec<RowKind>, Option<String>) {
    if let Some(mut combined) = state.azure_live_combined.take() {
        let (merged, message) = combined.absorb(kind, candidates, message);
        // まだ応答待ちの種別があれば集約を戻して次の到着に備える
        if combined.pending > 0 {
            state.azure_live_combined = Some(combined);
        }
        state.results = merged;
        state.empty_message = message;
    } else {
        state.results = candidates;
        state.empty_message = message;
    }
    let (labels, rows) = build_rows(&state.results, &[]);
    state.rows = if rows.is_empty() {
        vec![RowKind::Message]
    } else {
        rows.clone()
    };
    (labels, rows, state.empty_message.clone())
}

pub(super) fn set_azure_empty_message(message: &str) {
    let outcome = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.empty_message = Some(message.to_string());
        state.rows = vec![RowKind::Message];
        state.list
    });
    if let Some(list) = outcome {
        populate_empty_message(list, Some(message));
    }
}

pub(super) fn handle_azure_work_item_results(reply_id: u32) {
    let Some(reply) = crate::azure_devops::take_work_item_results(reply_id) else {
        return;
    };
    let outcome = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .azure_live_search_gate
            .finish(LiveSearchKind::WorkItems, Instant::now());
        if !accepts_azure_work_item_reply(
            state.azure_work_items_active,
            state.azure_work_item_reply_id,
            reply_id,
        ) {
            return None;
        }
        state.azure_work_items_active = false;
        let fetched_entries: Vec<Entry> = reply
            .candidates
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .map(|candidate| Entry {
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        state.index.merge_cached_work_items(&fetched_entries);
        let (labels, rows, empty_message) = apply_live_results(
            &mut state,
            LiveSearchKind::WorkItems,
            fetched_entries,
            reply.message,
        );
        Some((state.list, labels, rows, empty_message))
    });
    let Some((list, labels, rows, empty_message)) = outcome else {
        return;
    };
    if let Some(list) = list {
        if !rows.is_empty() {
            populate_list(list, &labels, &rows);
        } else {
            populate_empty_message(list, empty_message.as_deref());
        }
    }
}

/// PR 検索がキャッシュで 0 件だったときにリストへ足す、ライブ検索への入口。
pub(super) fn live_pull_request_search_entry(
    filter: crate::quick_launch::PullRequestFilter,
    query: &str,
) -> Entry {
    let label = if query.is_empty() {
        "Search Azure DevOps for older pull requests".to_string()
    } else {
        format!("Search Azure DevOps for pull requests matching \"{query}\"")
    };
    Entry {
        name: label,
        breadcrumb: "Not in cache — press Enter to search live (widens to 1 year)".to_string(),
        path: String::new(),
        action: crate::quick_launch::Action::AzureLivePullRequestSearch {
            filter,
            query: query.to_string(),
        },
        branch: None,
    }
}

/// `AzureLivePullRequestSearch` が選ばれた。ウィンドウは閉じずにその場で
/// API 検索を投げ、結果が届いたらリストだけ差し替える。
pub(super) fn start_azure_pull_request_live_search(
    state: &RefCell<State>,
    filter: crate::quick_launch::PullRequestFilter,
    query: &str,
) -> bool {
    let filter = filter.for_live();
    let key = format!("pr:{:?}:{}:{}", filter.status, filter.mine, query.trim());
    if !state.borrow_mut().azure_live_search_gate.try_start(
        LiveSearchKind::PullRequests,
        &key,
        Instant::now(),
    ) {
        return false;
    }
    let (window, reply_id, settings) = {
        let mut state = state.borrow_mut();
        // 集約モード (`az <query>`) では 3 種を同時に投げるので、他種別の
        // 実行中フラグや到着済みの結果を潰してはいけない
        if state.azure_live_combined.is_none() {
            invalidate_azure_live_searches(&mut state);
            state.results.clear();
        }
        state.azure_pull_requests_live_active = true;
        state.azure_pull_request_reply_id = next_azure_reply_id(state.azure_pull_request_reply_id);
        state.highlight_term.clear();
        // 集約モードでは 3 種の進捗を 1 行にまとめる (種別ごとの文言で
        // 上書きし合うと、どの検索が走っているか読めなくなる)
        state.empty_message = Some(if state.azure_live_combined.is_some() {
            "Searching Azure DevOps…".to_string()
        } else {
            "Searching Azure DevOps pull requests…".to_string()
        });
        state.rows = vec![RowKind::Message];
        (
            state.window,
            state.azure_pull_request_reply_id,
            state.azure_devops.clone(),
        )
    };
    let Some(list) = STATE.with(|state| state.borrow().list) else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::PullRequests, Instant::now());
        return false;
    };
    let progress = STATE.with(|state| state.borrow().empty_message.clone());
    populate_empty_message(list, progress.as_deref());
    let Some(window) = window else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::PullRequests, Instant::now());
        return false;
    };
    if !settings.enabled {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::PullRequests, Instant::now());
        set_azure_empty_message("Azure DevOps search is disabled in Settings.");
        return false;
    }
    crate::azure_devops::search_pull_requests_live_async(
        settings,
        filter.status.live_search_statuses(),
        filter.mine,
        query.to_string(),
        reply_id,
        window,
        WM_QUICK_LAUNCH_AZURE_RESULTS,
    );
    true
}

pub(super) fn accepts_azure_work_item_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

pub(super) fn accepts_azure_pull_request_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

pub(super) fn handle_azure_pull_request_results(reply_id: u32) {
    let Some(reply) = crate::azure_devops::take_pull_request_results(reply_id) else {
        return;
    };
    let outcome = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .azure_live_search_gate
            .finish(LiveSearchKind::PullRequests, Instant::now());
        if !accepts_azure_pull_request_reply(
            state.azure_pull_requests_live_active,
            state.azure_pull_request_reply_id,
            reply_id,
        ) {
            return None;
        }
        state.azure_pull_requests_live_active = false;
        let fetched_entries: Vec<Entry> = reply
            .candidates
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .map(|candidate| Entry {
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        let (labels, rows, empty_message) = apply_live_results(
            &mut state,
            LiveSearchKind::PullRequests,
            fetched_entries,
            reply.message,
        );
        Some((state.list, labels, rows, empty_message))
    });
    let Some((list, labels, rows, empty_message)) = outcome else {
        return;
    };
    if let Some(list) = list {
        if !rows.is_empty() {
            populate_list(list, &labels, &rows);
        } else {
            populate_empty_message(list, empty_message.as_deref());
        }
    }
}

/// Pipeline は永続キャッシュを持たないので、`az pipeline ` に入ると常に
/// このライブ検索への入口を出す (PR/Work Item の「キャッシュ 0 件」とは
/// 条件が異なるが、表示・選択の形は同じ)。
pub(super) fn live_pipeline_search_entry(
    filter: crate::quick_launch::PipelineFilter,
    query: &str,
) -> Entry {
    let label = if query.is_empty() {
        "Search Azure DevOps pipelines".to_string()
    } else {
        format!("Search Azure DevOps pipelines matching \"{query}\"")
    };
    Entry {
        name: label,
        breadcrumb: "Press Enter to search live".to_string(),
        path: String::new(),
        action: crate::quick_launch::Action::AzureLivePipelineSearch {
            filter,
            query: query.to_string(),
        },
        branch: None,
    }
}

/// `AzureLivePipelineSearch` が選ばれた。ウィンドウは閉じずにその場で
/// API 検索を投げ、結果が届いたらリストだけ差し替える。
pub(super) fn start_azure_pipeline_live_search(
    state: &RefCell<State>,
    filter: crate::quick_launch::PipelineFilter,
    query: &str,
) -> bool {
    let key = format!("pipeline:{filter:?}:{}", query.trim());
    if !state.borrow_mut().azure_live_search_gate.try_start(
        LiveSearchKind::Pipelines,
        &key,
        Instant::now(),
    ) {
        return false;
    }
    let (window, reply_id, settings) = {
        let mut state = state.borrow_mut();
        // 集約モード (`az <query>`) では 3 種を同時に投げるので、他種別の
        // 実行中フラグや到着済みの結果を潰してはいけない
        if state.azure_live_combined.is_none() {
            invalidate_azure_live_searches(&mut state);
            state.results.clear();
        }
        state.azure_pipelines_live_active = true;
        state.azure_pipeline_reply_id = next_azure_reply_id(state.azure_pipeline_reply_id);
        // 集約モードでは 3 種の進捗を 1 行にまとめる (種別ごとの文言で
        // 上書きし合うと、どの検索が走っているか読めなくなる)
        state.empty_message = Some(if state.azure_live_combined.is_some() {
            "Searching Azure DevOps…".to_string()
        } else {
            "Searching Azure DevOps pipelines…".to_string()
        });
        state.rows = vec![RowKind::Message];
        (
            state.window,
            state.azure_pipeline_reply_id,
            state.azure_devops.clone(),
        )
    };
    let Some(list) = STATE.with(|state| state.borrow().list) else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::Pipelines, Instant::now());
        return false;
    };
    let progress = STATE.with(|state| state.borrow().empty_message.clone());
    populate_empty_message(list, progress.as_deref());
    let Some(window) = window else {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::Pipelines, Instant::now());
        return false;
    };
    if !settings.enabled {
        state
            .borrow_mut()
            .azure_live_search_gate
            .finish(LiveSearchKind::Pipelines, Instant::now());
        set_azure_empty_message("Azure DevOps search is disabled in Settings.");
        return false;
    }
    crate::azure_devops::search_pipelines_live_async(
        settings,
        filter,
        query.to_string(),
        reply_id,
        window,
        WM_QUICK_LAUNCH_AZURE_RESULTS,
    );
    true
}

/// `Ctrl+Enter` を現在の `az` サブコマンドに対応する Live 検索へ振り分ける。
pub(super) fn start_azure_live_search_for_query(
    state: &RefCell<State>,
    query: &str,
) -> AzureLiveSearchStart {
    let Some(request) = crate::quick_launch::azure_live_request(query) else {
        return AzureLiveSearchStart::NotApplicable;
    };
    let started = match request {
        crate::quick_launch::AzureLiveRequest::AllKinds { query } => {
            start_azure_combined_live_search(state, &query)
        }
        crate::quick_launch::AzureLiveRequest::WorkItems { query } => {
            start_azure_work_item_live_search(state, &query)
        }
        crate::quick_launch::AzureLiveRequest::PullRequests { filter, query } => {
            start_azure_pull_request_live_search(state, filter, &query)
        }
        crate::quick_launch::AzureLiveRequest::Pipelines { filter, query } => {
            start_azure_pipeline_live_search(state, filter, &query)
        }
    };
    if started {
        AzureLiveSearchStart::Started
    } else {
        AzureLiveSearchStart::Suppressed
    }
}

/// `az <query>` (サブコマンド無し) の `Ctrl+Enter`。PR / Work Item /
/// Pipeline の 3 種を同時に投げ、到着順に結果をマージして表示する
/// (横断検索の一覧と Live 検索の対象を揃えるため)。
///
/// 集約バッファを先に置いてから各 start を呼ぶ。start 側はこのバッファの
/// 有無で「他種別を潰さない」「進捗文言を共通化する」を切り替える。
pub(super) fn start_azure_combined_live_search(state: &RefCell<State>, query: &str) -> bool {
    {
        let mut state = state.borrow_mut();
        invalidate_azure_live_searches(&mut state);
        state.results.clear();
        state.highlight_term.clear();
        state.azure_live_combined = Some(CombinedLiveSearch::new(3));
    }
    let started = [
        start_azure_pull_request_live_search(
            state,
            crate::quick_launch::PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Active,
                status_explicit: false,
                mine: false,
                live: true,
            },
            query,
        ),
        start_azure_work_item_live_search(state, query),
        start_azure_pipeline_live_search(state, crate::quick_launch::PipelineFilter::All, query),
    ];
    let launched = started.iter().filter(|started| **started).count();
    let mut state_ref = state.borrow_mut();
    if launched == 0 {
        // 全種が gate に弾かれた / 設定無効。集約は残さない
        state_ref.azure_live_combined = None;
        return false;
    }
    // 投げられなかった種別の分だけ待ち数を減らす (減らさないと
    // 「Searching…」のまま最後の応答で確定しない)
    if let Some(combined) = state_ref.azure_live_combined.as_mut() {
        combined.pending = launched;
    }
    true
}

/// 入力が変わった、または別種別の Live 検索を始めるときに、先行応答を
/// 現在の一覧へ反映させない。3 種別は reply_id が独立しているため、
/// 個別フラグを残すと別種別の遅延応答まで有効になってしまう。
pub(super) fn invalidate_azure_live_searches(state: &mut State) {
    state.azure_work_items_active = false;
    state.azure_pull_requests_live_active = false;
    state.azure_pipelines_live_active = false;
}

pub(super) fn accepts_azure_pipeline_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

pub(super) fn handle_azure_pipeline_results(reply_id: u32) {
    let Some(reply) = crate::azure_devops::take_pipeline_results(reply_id) else {
        return;
    };
    let outcome = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .azure_live_search_gate
            .finish(LiveSearchKind::Pipelines, Instant::now());
        if !accepts_azure_pipeline_reply(
            state.azure_pipelines_live_active,
            state.azure_pipeline_reply_id,
            reply_id,
        ) {
            return None;
        }
        state.azure_pipelines_live_active = false;
        let fetched_entries: Vec<Entry> = reply
            .candidates
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .map(|candidate| Entry {
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        let (labels, rows, empty_message) = apply_live_results(
            &mut state,
            LiveSearchKind::Pipelines,
            fetched_entries,
            reply.message,
        );
        Some((state.list, labels, rows, empty_message))
    });
    let Some((list, labels, rows, empty_message)) = outcome else {
        return;
    };
    if let Some(list) = list {
        if !rows.is_empty() {
            populate_list(list, &labels, &rows);
        } else {
            populate_empty_message(list, empty_message.as_deref());
        }
    }
}

