//! Outlook メール検索のデバウンスと非同期応答の UI 反映。

use std::cell::RefCell;

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::search::{build_rows, populate_empty_message, populate_list};
use super::{
    MAX_LIST_RESULTS, OUTLOOK_SEARCH_DELAY_MS, OUTLOOK_TIMER_ID, RowKind, STATE, State,
    WM_QUICK_LAUNCH_OUTLOOK_RESULTS,
};
use crate::quick_launch::Entry;

/// `o ` プレフィックスに入った。毎打鍵で Node.js の CLI を起動しないよう、
/// 最後の入力から 250ms 後に 1 回だけ検索を起動する。タイマーを再設定する前に
/// 要求 ID を進めるため、既に起動済みの古い応答も到着時に捨てられる。
pub(super) fn start_outlook_query(state: &RefCell<State>, text: &str) {
    let (window, list, message) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        state.previous_query = None;
        state.highlight_term.clear();
        state.outlook_active = true;
        state.outlook_reply_id = next_outlook_reply_id(state.outlook_reply_id);
        state.outlook_query = text.trim().to_string();
        state.results.clear();
        state.rows = vec![RowKind::Message];
        state.empty_message = Some(if state.outlook_query.is_empty() {
            "Type to search Outlook mail.".to_string()
        } else {
            "Searching Outlook…".to_string()
        });
        (state.window, state.list, state.empty_message.clone())
    };
    let Some(list) = list else {
        return;
    };
    if let Some(window) = window {
        unsafe {
            let _ = KillTimer(Some(window), OUTLOOK_TIMER_ID);
            if !text.trim().is_empty() {
                let _ = SetTimer(
                    Some(window),
                    OUTLOOK_TIMER_ID,
                    OUTLOOK_SEARCH_DELAY_MS,
                    None,
                );
            }
        }
    }
    populate_empty_message(list, message.as_deref());
}

/// デバウンスタイマーの通知から、現在の 1 件だけを CLI ワーカーへ渡す。
pub(super) fn start_outlook_search() {
    let Some((window, request_id, query)) = STATE.with(|state| {
        let state = state.borrow();
        (state.outlook_active && !state.outlook_query.is_empty()).then_some((
            state.window?,
            state.outlook_reply_id,
            state.outlook_query.clone(),
        ))
    }) else {
        return;
    };
    crate::outlook::search_async(query, request_id, window, WM_QUICK_LAUNCH_OUTLOOK_RESULTS);
}

/// Outlook CLI ワーカーの応答を現在の `o ` クエリにだけ反映する。
pub(super) fn handle_outlook_results(reply_id: u32) {
    let Some(reply) = crate::outlook::take_search_reply(reply_id) else {
        return;
    };
    let outcome = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !accepts_outlook_reply(state.outlook_active, state.outlook_reply_id, reply_id) {
            return None;
        }
        state.results = reply
            .messages
            .into_iter()
            .take(MAX_LIST_RESULTS)
            .map(|message| Entry {
                name: message.subject,
                breadcrumb: if message.received.is_empty() {
                    message.sender
                } else {
                    format!("{} — {}", message.sender, message.received)
                },
                path: message.web_link.clone(),
                action: crate::quick_launch::Action::OpenUrl(message.web_link),
                branch: None,
            })
            .collect();
        state.empty_message = reply.error.or_else(|| {
            state
                .results
                .is_empty()
                .then(|| "No Outlook messages found.".to_string())
        });
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = if rows.is_empty() {
            vec![RowKind::Message]
        } else {
            rows.clone()
        };
        Some((state.list, labels, rows, state.empty_message.clone()))
    });
    let Some((list, labels, rows, message)) = outcome else {
        return;
    };
    if let Some(list) = list {
        if rows.is_empty() {
            populate_empty_message(list, message.as_deref());
        } else {
            populate_list(list, &labels, &rows);
        }
    }
}

pub(super) fn next_outlook_reply_id(current: u32) -> u32 {
    current.checked_add(1).filter(|id| *id != 0).unwrap_or(1)
}

pub(super) fn accepts_outlook_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}
