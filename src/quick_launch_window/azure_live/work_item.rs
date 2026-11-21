//! `az wit` — Work Item のキャッシュ検索とライブ検索。

use std::cell::RefCell;
use std::time::Instant;

use super::super::search::{
    build_rows, next_azure_reply_id, populate_empty_message, populate_list,
};
use super::super::{MAX_LIST_RESULTS, RowKind, STATE, State, WM_QUICK_LAUNCH_AZURE_RESULTS};
use super::{
    LiveSearchKind, apply_live_results, invalidate_azure_live_searches, set_azure_empty_message,
};
use crate::quick_launch::Entry;

/// `az wit ` に入った。SQLite キャッシュ (バックグラウンド定期同期が
/// 埋める「最近更新された Work Item」) だけをローカル検索する。API は
/// 自動では呼ばない — 検索語があってキャッシュに無ければ、末尾に
/// 明示的なライブ検索の選択肢を 1 件足すだけに留める (ユーザー操作が
/// トリガー、入力のたびに Azure DevOps を叫ばない)。
pub(in crate::quick_launch_window) fn start_azure_work_item_query(
    state: &RefCell<State>,
    filter: crate::quick_launch::WorkItemFilter,
    text: &str,
) {
    let (list, has_results) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        // キャッシュ検索へ戻った時点で、打ち切られた集約 Live 検索の
        // 残りも捨てる (`invalidate_azure_live_searches` と同じ理由)。
        state.azure_live_combined = None;
        state.previous_query = None;
        state.highlight_term = crate::quick_launch::azure_highlight_term(text.trim()).to_string();
        state.results = state
            .index
            .search_cached_work_items(filter, text)
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .cloned()
            .collect();
        let trimmed = text.trim();
        if state.results.is_empty() && (!trimmed.is_empty() || filter.is_filtered()) {
            state
                .results
                .push(live_work_item_search_entry(filter, trimmed));
        }
        state.empty_message = if state.results.is_empty() {
            Some(if trimmed.is_empty() {
                "No recently updated work items.".to_string()
            } else {
                "No matching work items.".to_string()
            })
        } else {
            Some(if trimmed.is_empty() {
                "Cached work items. Ctrl+Enter: Area/Iteration scope, up to 800 per project."
            } else if crate::azure_devops::exact_id_query(trimmed).is_some() {
                "Cached work items by ID. Ctrl+Enter: fetch that ID in watched projects."
            } else {
                "Cached work items. Ctrl+Enter: full-project text search (up to 300 per project)."
            }.into())
        };
        (state.list, !state.results.is_empty())
    };
    let Some(list) = list else {
        return;
    };
    let (labels, rows, message) = {
        let mut state = state.borrow_mut();
        let (labels, rows) = build_rows(&state.results, &[]);
        let (labels, rows) =
            super::super::list::with_message(labels, rows, state.empty_message.as_deref());
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

/// キャッシュ検索が 0 件だったときにリストへ足す、ライブ検索への入口。
fn live_work_item_search_entry(filter: crate::quick_launch::WorkItemFilter, query: &str) -> Entry {
    Entry {
        azure: None,
        name: if query.is_empty() {
            "Search Azure DevOps for matching work items".to_string()
        } else {
            format!("Search Azure DevOps for \"{query}\"")
        },
        breadcrumb: "Not in cache — press Enter to search live".to_string(),
        path: String::new(),
        action: crate::quick_launch::Action::AzureLiveWorkItemSearch {
            filter,
            query: query.to_string(),
        },
        branch: None,
    }
}

/// `AzureLiveWorkItemSearch` が選ばれた。ウィンドウは閉じずにその場で
/// API 検索を投げ、結果が届いたらリストだけ差し替える。
pub(in crate::quick_launch_window) fn start_azure_work_item_live_search(
    state: &RefCell<State>,
    filter: crate::quick_launch::WorkItemFilter,
    query: &str,
) -> bool {
    let key = format!("wit:{filter:?}:{}", query.trim());
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
        state.azure_work_item_filter = filter;
        // 集約モードは PR / Pipeline と一覧を共有するのでハイライトを付けない
        if state.azure_live_combined.is_none() {
            state.highlight_term =
                crate::quick_launch::azure_highlight_term(query.trim()).to_string();
        }
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

pub(in crate::quick_launch_window) fn accepts_azure_work_item_reply(
    active: bool,
    expected: u32,
    received: u32,
) -> bool {
    active && expected == received
}

pub(in crate::quick_launch_window) fn handle_azure_work_item_results(reply_id: u32) {
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
                azure: Some(crate::quick_launch::AzureMeta::from_candidate(&candidate)),
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        state.index.merge_cached_work_items(&fetched_entries);
        // 属性 (`mine` / `bug` 等) は API に渡さず、取得した結果をここで絞る。
        // キャッシュには絞る前の全件を入れる (別の属性で引き直したときに使う)
        let filter = state.azure_work_item_filter;
        let fetched = fetched_entries.len();
        let filtered: Vec<Entry> = fetched_entries
            .into_iter()
            .filter(|entry| filter.matches(entry.azure.as_ref()))
            .collect();
        let message = filtered_live_message(fetched, filtered.len(), reply.message);
        let (labels, rows, empty_message) =
            apply_live_results(&mut state, LiveSearchKind::WorkItems, filtered, message);
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

/// 取得した結果が属性で全部落ちたときは、取得範囲の文言の前にその旨を足す。
/// 取得 0 件のときは API 側の文言 ("No matching ...") をそのまま使う。
pub(in crate::quick_launch_window) fn filtered_live_message(
    fetched: usize,
    kept: usize,
    message: Option<String>,
) -> Option<String> {
    if fetched == 0 || kept > 0 {
        return message;
    }
    let lead = format!("None of the {fetched} fetched work items match the attributes.");
    Some(match message {
        Some(message) => format!("{lead} — {message}"),
        None => lead,
    })
}

/// `az <番号>` 用。その番号の Work Item が一覧に無いか (PR の同じ番号は数えない)。
pub(in crate::quick_launch_window) fn missing_azure_work_item_id(
    results: &[Entry],
    query: &str,
) -> bool {
    crate::azure_devops::is_item_id_query(query)
        && !results.iter().any(|entry| {
            entry
                .azure
                .as_ref()
                .is_some_and(|meta| meta.kind == crate::azure_devops::Kind::WorkItem)
                && crate::azure_devops::matches_item_id(&entry.name, query)
        })
}

/// `az <番号>` で、その番号の Work Item を ID で直接引く入口。`#<番号>` として
/// Live 検索へ渡すので、全文検索ではなく ID 完全一致の取得になる。
pub(in crate::quick_launch_window) fn work_item_lookup_entry(query: &str) -> Entry {
    let id = query.trim();
    Entry {
        azure: None,
        name: format!("Look up Work Item {id} on Azure DevOps"),
        breadcrumb: "Not in cache — press Enter to fetch it by number".to_string(),
        path: String::new(),
        action: crate::quick_launch::Action::AzureLiveWorkItemSearch {
            filter: crate::quick_launch::WorkItemFilter::default(),
            query: format!("#{id}"),
        },
        branch: None,
    }
}
