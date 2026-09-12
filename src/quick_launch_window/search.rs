//! 検索実行・非同期結果の反映。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, LB_ADDSTRING, LB_RESETCONTENT, LB_SETCURSEL,
};
use windows::core::HSTRING;

use super::layout::{position_window, rows_height};
use super::{
    EDIT_HEIGHT, EVERYTHING_MAX_RESULTS, EVERYTHING_REPLY_ID_START, MAX_LIST_RESULTS, PADDING,
    RowKind, STATE, State, WM_QUICK_LAUNCH_AZURE_RESULTS,
};
use crate::config::OpenMode;
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

/// 検索結果を作り直し、リストボックスへ反映する。
///
/// `SendMessageW` は同期呼び出しで、リストボックスは所有者へ
/// `WM_ERASEBKGND` / `WM_DRAWITEM` をその場で送り返す。再入した
/// `window_proc` は STATE を `borrow()` するため、**借用を保持したまま
/// Win32 を呼んではいけない** (保持すると BorrowMutError で panic し、
/// `window_proc` は unwind 不可なので abort する)。
/// 借用中は検索と `results` の更新だけを行い、描画用の値を取り出してから
/// 借用を解放し、その後で `SendMessageW` を呼ぶ。
/// 検索窓に出すモードバッジを入力文字列から判定し、変わっていれば
/// 検索窓部分だけ再描画する。
pub(super) fn update_badge(state: &RefCell<State>, query: &str) {
    let badge = crate::quick_launch::prefix_badge(query);
    let live_hint = crate::quick_launch::azure_live_request(query).is_some();
    let (window, dpi, changed) = {
        let mut state = state.borrow_mut();
        let changed = state.badge != badge || state.live_search_hint != live_hint;
        state.badge = badge;
        state.live_search_hint = live_hint;
        (state.window, state.dpi, changed)
    };
    if changed {
        invalidate_search_bar(window, dpi);
    }
}

/// 検索窓 (バッジを含む上部の帯) だけを再描画対象にする。
/// リスト部分を巻き込まないことで、バッジ更新のたびにリスト全体が
/// ちらつくのを防ぐ。
pub(super) fn invalidate_search_bar(window: Option<HWND>, dpi: u32) {
    let Some(window) = window else {
        return;
    };
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        let search_rect = RECT {
            left: 0,
            top: 0,
            right: client.right,
            bottom: super::layout::scale(PADDING, dpi) * 2 + super::layout::scale(EDIT_HEIGHT, dpi),
        };
        let _ = InvalidateRect(Some(window), Some(&search_rect), false);
    }
}

pub(super) fn update_results(state: &RefCell<State>) {
    // read_text も Win32 呼び出しなので借用の外で済ませる
    let edit = state.borrow().edit;
    let query = edit.map(super::input::read_text).unwrap_or_default();
    // 入力が変わった時点で、先行する Live 検索の応答は現在の候補ではない。
    // リクエスト自体は中断できないため、到着時に捨てる。
    invalidate_azure_live_searches(&mut state.borrow_mut());
    update_badge(state, &query);

    if let Some(rest) = query.strip_prefix(crate::quick_launch::EVERYTHING_PREFIX) {
        start_everything_query(state, rest);
        return;
    }
    if let Some((crate::quick_launch::AzureCommand::WorkItems { .. }, rest)) =
        crate::quick_launch::azure_command(&query)
    {
        start_azure_work_item_query(state, rest);
        return;
    }
    if let Some((crate::quick_launch::AzureCommand::Suggest, _)) =
        crate::quick_launch::azure_command(&query)
    {
        show_azure_suggest_entry(state);
        return;
    }
    let (list, labels, rows, copy_feedback_cleared, window, dpi) = {
        let mut state = state.borrow_mut();
        // プレフィックスを外れたら、遅れて届く Everything の応答を無視させる
        state.everything_active = false;
        state.empty_message = None;
        // COPIED バッジは update_badge のプレフィックスバッジ変化検知の
        // 対象外なので、ここで変化を見て自前で再描画要求しないと、次に
        // 何か別の理由で再描画されるまで古い COPIED 表示が残り続ける。
        let copy_feedback_cleared = state.copy_feedback;
        state.copy_feedback = false;
        state.highlight_term = if query.is_empty() {
            String::new()
        } else {
            crate::quick_launch::effective_search_term(&query).to_string()
        };
        let section_headers = if query.is_empty() {
            // 絞り込みなし: Spotlight 風に区分見出し付きで一覧を組み立てる
            let mut results = Vec::new();
            let mut section_headers = Vec::new();
            for (label, entries) in state.index.sections() {
                section_headers.push((results.len(), label));
                results.extend(entries.into_iter().cloned());
            }
            state.results = results;
            section_headers
        } else {
            // `state.results` が前回の表示上限 (MAX_LIST_RESULTS) で切り詰め
            // 済みだと、絞り込みで再び上位に来るはずの候補 (切り詰めで
            // 落ちた 25 件目以降) が母集団に無く拾えない。切り詰めが
            // 起きていた回だけは安全側に倒し、全候補への再検索にフォールバック
            // する (実測: 候補 25 件超のとき、絞り込みを続けると本来ヒット
            // するはずの候補が一覧から消えていた)。
            state.results = if let Some(search_term) =
                refinable_search_term(state.previous_query.as_deref(), &query, state.results.len())
            {
                crate::quick_launch::search_entries(
                    &state.results,
                    search_term,
                    state.index.search_paths,
                    &state.index.ranking,
                )
                .into_iter()
                .take(MAX_LIST_RESULTS)
                .cloned()
                .collect()
            } else {
                state
                    .index
                    .search(&query)
                    .into_iter()
                    .take(MAX_LIST_RESULTS)
                    .cloned()
                    .collect()
            };
            // PR 検索がキャッシュで 0 件だったとき、末尾に明示的なライブ
            // 検索の入口を足す。API 全文検索が無いため、打ち切り期間を
            // 一時的に広げて再取得する以外に取りこぼしを拾う手段が無い。
            if state.results.is_empty()
                && let Some((crate::quick_launch::AzureCommand::PullRequests(filter), rest)) =
                    crate::quick_launch::azure_command(&query)
            {
                state
                    .results
                    .push(live_pull_request_search_entry(filter, rest));
            }
            // Pipeline は永続キャッシュを持たないので、`Index::search` は
            // 常に 0 件を返す (`quick_launch/search.rs` 参照)。ここで毎回
            // ライブ検索への入口を足す — PR/Work Item と違い「キャッシュに
            // 無かった」ではなく最初から Live 検索専用のため無条件。
            if state.results.is_empty()
                && let Some((crate::quick_launch::AzureCommand::Pipelines(filter), rest)) =
                    crate::quick_launch::azure_command(&query)
            {
                state.results.push(live_pipeline_search_entry(filter, rest));
            }
            Vec::new()
        };
        state.previous_query = Some(query);
        let (labels, rows) = build_rows(&state.results, &section_headers);
        state.rows = rows.clone();
        (
            state.list,
            labels,
            rows,
            copy_feedback_cleared,
            state.window,
            state.dpi,
        )
    }; // ← ここで借用が切れる。以降の再入は borrow() できる

    if copy_feedback_cleared {
        invalidate_search_bar(window, dpi);
    }

    let Some(list) = list else {
        return;
    };
    populate_list(list, &labels, &rows);
}

/// `f ` プレフィックスに入った。Everything へ非同期クエリを送り、
/// 結果が届くまでの間はリストを空にする。
///
/// Everything 未起動・設定で無効の場合は何も送らず空のまま。
/// 空の検索語 (`f ` だけ) はクエリを送らない — 全件検索は重く、
/// タイプの途中で毎回投げると Everything 側の応答待ちが積み上がる。
pub(super) fn start_everything_query(state: &RefCell<State>, text: &str) {
    let (window, list, enabled, flags, reply_id) = {
        let mut state = state.borrow_mut();
        state.everything_active = true;
        state.previous_query = None;
        state.highlight_term.clear();
        state.results.clear();
        state.rows.clear();
        state.everything_reply_id = next_everything_reply_id(state.everything_reply_id);
        (
            state.window,
            state.list,
            state.everything_enabled,
            state.everything_flags,
            state.everything_reply_id,
        )
    };
    if let Some(list) = list {
        populate_list(list, &[], &[]);
    }
    let (Some(window), true, false) = (window, enabled, text.is_empty()) else {
        return;
    };
    crate::everything::query(window, reply_id, text, EVERYTHING_MAX_RESULTS, flags);
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
fn show_azure_suggest_entry(state: &RefCell<State>) {
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
fn live_pull_request_search_entry(
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
fn live_pipeline_search_entry(filter: crate::quick_launch::PipelineFilter, query: &str) -> Entry {
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

/// Everything から届いた `WM_COPYDATA` を結果リストへ反映する。
///
/// `f ` を抜けていれば `update_results` が `everything_active` を
/// 下ろしているので、届いた結果はここでは扱わず捨てる (プレフィックスが
/// 外れた後に遅延到着した応答が、無関係な検索結果へ紛れ込むのを防ぐ)。
pub(super) fn handle_everything_results(reply_id: u32, data: &[u8]) {
    let parsed = crate::everything::parse_results(data);
    let outcome = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if !accepts_everything_reply(state.everything_active, state.everything_reply_id, reply_id) {
            return None;
        }
        state.results = parsed
            .into_iter()
            .map(|item| crate::quick_launch::Entry {
                name: item.name,
                breadcrumb: String::new(),
                path: item.path,
                action: if item.is_folder {
                    crate::quick_launch::Action::OpenFolder(OpenMode::NewWindow)
                } else {
                    crate::quick_launch::Action::OpenWithDefaultHandler
                },
                branch: None,
            })
            .collect();
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = rows.clone();
        Some((state.list, labels, rows))
    });
    let Some((list, labels, rows)) = outcome else {
        return;
    };
    if let Some(list) = list {
        populate_list(list, &labels, &rows);
    }
}

pub(super) fn next_everything_reply_id(current: u32) -> u32 {
    current
        .checked_add(1)
        .filter(|next| *next >= EVERYTHING_REPLY_ID_START)
        .unwrap_or(EVERYTHING_REPLY_ID_START)
}

pub(super) fn next_azure_reply_id(current: u32) -> u32 {
    current.checked_add(1).filter(|id| *id != 0).unwrap_or(1)
}

pub(super) fn accepts_everything_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

pub(super) fn accepts_azure_work_item_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

/// `refined_search_term` の結果に、前回の結果件数による安全条件を足す。
///
/// `state.results` は表示上限 (`MAX_LIST_RESULTS`) で切り詰め済みのことが
/// ある。切り詰めが起きていた回に前回の結果だけを母集団にすると、切り詰めで
/// 落ちた候補 (絞り込みで本来上位に来るはずのもの) を拾えない。切り詰めが
/// 起きていない (`previous_results_len < MAX_LIST_RESULTS`) ときだけ最適化を
/// 使い、それ以外は全候補への再検索に倒す。
pub(super) fn refinable_search_term<'a>(
    previous_query: Option<&str>,
    current: &'a str,
    previous_results_len: usize,
) -> Option<&'a str> {
    if previous_results_len >= MAX_LIST_RESULTS {
        return None;
    }
    refined_search_term(previous_query?, current)
}

/// 前回の検索結果だけを対象にしても漏れがない場合の、今回の検索語を返す。
///
/// 通常検索と `b ` / `h ` / `w ` / `a ` の同一モードでは、入力末尾への文字追加で
/// 一致集合が広がらない。Everything と Azure DevOps は別経路なので対象外にする。
fn refined_search_term<'a>(previous: &str, current: &'a str) -> Option<&'a str> {
    let (previous_scope, previous_term) = local_search_scope(previous)?;
    let (current_scope, current_term) = local_search_scope(current)?;
    (previous_scope == current_scope
        && !previous_term.is_empty()
        && current_term.len() > previous_term.len()
        && current_term.starts_with(previous_term))
    .then_some(current_term)
}

fn local_search_scope(query: &str) -> Option<(&'static str, &str)> {
    if query.starts_with(crate::quick_launch::EVERYTHING_PREFIX)
        || query.starts_with(crate::quick_launch::AZURE_DEVOPS_PREFIX)
    {
        return None;
    }
    if let Some(term) = query.strip_prefix("b ") {
        Some(("bookmarks", term))
    } else if let Some(term) = query.strip_prefix("h ") {
        Some(("history", term))
    } else if let Some(term) = query.strip_prefix("w ") {
        Some(("windows", term))
    } else if let Some(term) = query.strip_prefix("a ") {
        Some(("apps", term))
    } else if let Some(term) = query.strip_prefix("t ") {
        Some(("tabs", term))
    } else if let Some(term) = query.strip_prefix("ps ") {
        Some(("terminal", term))
    } else {
        Some(("normal", query))
    }
}

/// リストボックスの中身を丸ごと差し替える。通常検索と Everything の
/// 非同期結果受信 (`handle_everything_results`) の双方から使う。
/// `rows` が空でも構わない (見出しも項目もない = 空一覧)。
/// 初期カーソルは、見出し行を飛ばした最初の項目行に置く。
///
/// 合わせてウィンドウの高さを `rows` の実際の行数へ合わせ直す。候補数が
/// 変わるたびに呼ぶことで、候補が少ないときに下部の空きリストボックス分の
/// 余白が残るのを防ぐ。
pub(super) fn populate_list(list: HWND, labels: &[HSTRING], rows: &[RowKind]) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_RESETCONTENT,
            None,
            None,
        );
        for label in labels {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        if let Some(row) = rows.iter().position(|row| matches!(row, RowKind::Item(_))) {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_SETCURSEL,
                Some(WPARAM(row)),
                None,
            );
        }
    }
    resize_to_rows(rows);
}

/// `populate_list` から呼ぶ、ウィンドウ再配置の実処理。表示中でなければ何もしない。
fn resize_to_rows(rows: &[RowKind]) {
    let (window, dpi, visible_results) = STATE.with(|state| {
        let state = state.borrow();
        (state.window, state.dpi, state.visible_results)
    });
    let Some(window) = window else {
        return;
    };
    let height = rows_height(rows, visible_results);
    position_window(window, height, dpi);
}

/// `results` と区分見出し (`results` 側インデックス昇順の `(挿入位置, ラベル)`) から、
/// リストボックスへ渡す行ラベルと `State::rows` を作る。
/// `section_headers` が空なら見出しなしのフラットな 1:1 対応になる。
pub(super) fn build_rows(
    results: &[Entry],
    section_headers: &[(usize, &'static str)],
) -> (Vec<HSTRING>, Vec<RowKind>) {
    let mut labels = Vec::with_capacity(results.len() + section_headers.len());
    let mut rows = Vec::with_capacity(results.len() + section_headers.len());
    let mut headers = section_headers.iter().peekable();
    for (index, entry) in results.iter().enumerate() {
        while let Some((at, label)) = headers.peek() {
            if *at != index {
                break;
            }
            labels.push(HSTRING::from(*label));
            rows.push(RowKind::Header(label));
            headers.next();
        }
        labels.push(HSTRING::from(format!(
            "{}    {}",
            entry.name,
            super::draw::entry_context(entry)
        )));
        rows.push(RowKind::Item(index));
    }
    // 末尾 (results が空、または最後の区分) に付く見出しも取りこぼさない
    for (_, label) in headers {
        labels.push(HSTRING::from(*label));
        rows.push(RowKind::Header(label));
    }
    (labels, rows)
}

/// 説明用の 1 行を出す。`results` には追加しないため Enter で実行されない。
/// 呼び出し元は `state.rows` もこの内容に合わせて更新すること
/// (古い `RowKind::Item` が残ると、表示上は消えた項目を選択・実行できてしまう)。
pub(super) fn populate_empty_message(list: HWND, message: Option<&str>) {
    let labels = message.map(HSTRING::from).into_iter().collect::<Vec<_>>();
    let rows = vec![RowKind::Message; labels.len()];
    populate_list(list, &labels, &rows);
}
