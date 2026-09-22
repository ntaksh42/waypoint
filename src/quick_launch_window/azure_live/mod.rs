//! Azure DevOps の Live 検索 (`az `、FR-9.18)。
//!
//! `search.rs` から分離し、さらに種別ごとの実装を子モジュールへ分けた。
//! ここには 3 種別が共有する gate・集約バッファ・結果反映と、
//! `Ctrl+Enter` の振り分けだけを置く。
//!
//! 元は `search.rs` から分離した。キャッシュ検索と違い API を非同期に叩き、
//! 応答を `WM_COPYDATA` 相当のメッセージで受けて一覧へ流し込むまでが
//! 一続きの関心事なので、検索窓そのものの処理とは別ファイルにする。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::HSTRING;

use super::search::{build_rows, populate_empty_message, populate_list};
use super::{MAX_LIST_RESULTS, RowKind, STATE, State};
use crate::quick_launch::Entry;

mod pipeline;
mod pull_request;
mod work_item;

// `accepts_azure_*_reply` は種別ごとのハンドラが自分で使うだけなので
// 再エクスポートしない。Work Item のものだけはテストが直接叩く。
pub(super) use pipeline::{
    handle_azure_pipeline_results, live_pipeline_search_entry, start_azure_pipeline_live_search,
};
pub(super) use pull_request::{
    handle_azure_pull_request_results, live_pull_request_search_entry,
    start_azure_pull_request_live_search,
};
pub(super) use work_item::{
    handle_azure_work_item_results, start_azure_work_item_live_search, start_azure_work_item_query,
};
// 応答 ID の照合はハンドラ内で完結するので通常ビルドでは再エクスポートしない。
// テストだけが直接叩く (stale な応答を捨てる条件の確認)。
#[cfg(test)]
pub(super) use work_item::accepts_azure_work_item_reply;

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

/// Live 検索の応答を `State` へ反映する。集約モード (`az <query>`) では
/// 種別ごとの結果をマージし、単独種別なら従来どおり丸ごと置き換える。
/// 戻り値は借用解放後にリストへ流し込むための (labels, rows, message)。
pub(super) fn apply_live_results(
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
                author: false,
                reviewer: false,
                needs_review: false,
                waiting: false,
                draft: false,
                ready: false,
                stale: false,
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
///
/// 集約バッファも同時に捨てる。捨てないと、応答が揃う前に入力が変わって
/// 打ち切られた `az <query>` の集約が `Some` のまま残り、次に単独種別の
/// Live 検索 (`az wit foo` など) を投げたときに前の検索の PR / Pipeline が
/// 結果へ混ざる (単独種別の start は集約が `Some` の間、一覧を消さず
/// 他種別の結果を温存する作りのため)。`az <query>` 自身はこの直後に
/// 新しい集約を置き直すので影響しない。
pub(super) fn invalidate_azure_live_searches(state: &mut State) {
    state.azure_work_items_active = false;
    state.azure_pull_requests_live_active = false;
    state.azure_pipelines_live_active = false;
    state.azure_live_combined = None;
}
