//! `az pipeline` — Pipeline のライブ検索 (永続キャッシュを持たない)。

use std::cell::RefCell;
use std::time::Instant;

use super::super::search::{next_azure_reply_id, populate_empty_message, populate_list};
use super::super::{MAX_LIST_RESULTS, RowKind, STATE, State, WM_QUICK_LAUNCH_AZURE_RESULTS};
use super::{
    LiveSearchKind, apply_live_results, invalidate_azure_live_searches, set_azure_empty_message,
};
use crate::quick_launch::Entry;

/// Pipeline は永続キャッシュを持たないので、`az pipeline ` に入ると常に
/// このライブ検索への入口を出す (PR/Work Item の「キャッシュ 0 件」とは
/// 条件が異なるが、表示・選択の形は同じ)。
pub(in crate::quick_launch_window) fn live_pipeline_search_entry(
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
pub(in crate::quick_launch_window) fn start_azure_pipeline_live_search(
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

pub(in crate::quick_launch_window) fn accepts_azure_pipeline_reply(
    active: bool,
    expected: u32,
    received: u32,
) -> bool {
    active && expected == received
}

pub(in crate::quick_launch_window) fn handle_azure_pipeline_results(reply_id: u32) {
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
