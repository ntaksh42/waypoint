//! 検索実行・非同期結果の反映。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, LB_ADDSTRING, LB_RESETCONTENT, LB_SETCURSEL,
};
use windows::core::HSTRING;

use super::{
    EDIT_HEIGHT, EVERYTHING_MAX_RESULTS, EVERYTHING_REPLY_ID_START, PADDING, RowKind, STATE,
    State, WM_QUICK_LAUNCH_AZURE_RESULTS,
};
use crate::config::OpenMode;
use crate::quick_launch::Entry;

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
    let (window, dpi, changed) = {
        let mut state = state.borrow_mut();
        let changed = state.badge != badge;
        state.badge = badge;
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
    update_badge(state, &query);

    if let Some(rest) = query.strip_prefix(crate::quick_launch::EVERYTHING_PREFIX) {
        start_everything_query(state, rest);
        return;
    }
    if let Some((crate::quick_launch::AzureCommand::WorkItems, rest)) =
        crate::quick_launch::azure_command(&query)
    {
        start_azure_work_item_query(state, rest);
        return;
    }

    let (list, labels, rows) = {
        let mut state = state.borrow_mut();
        // プレフィックスを外れたら、遅れて届く Everything の応答を無視させる
        state.everything_active = false;
        state.azure_work_items_active = false;
        state.empty_message = None;
        state.copy_feedback = false;
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
            state.results = if let Some(search_term) = state
                .previous_query
                .as_deref()
                .and_then(|previous| refined_search_term(previous, &query))
            {
                crate::quick_launch::search_entries(
                    &state.results,
                    search_term,
                    state.index.search_paths,
                    &state.index.ranking,
                )
                .into_iter()
                .cloned()
                .collect()
            } else {
                state.index.search(&query).into_iter().cloned().collect()
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
            Vec::new()
        };
        state.previous_query = Some(query);
        let (labels, rows) = build_rows(&state.results, &section_headers);
        state.rows = rows.clone();
        (state.list, labels, rows)
    }; // ← ここで借用が切れる。以降の再入は borrow() できる

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
        state.azure_work_items_active = true;
        state.results = state
            .index
            .search_cached_work_items(text)
            .into_iter()
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
pub(super) fn start_azure_work_item_live_search(state: &RefCell<State>, query: &str) {
    let (window, reply_id, settings) = {
        let mut state = state.borrow_mut();
        state.azure_work_items_active = true;
        state.azure_work_item_reply_id = next_azure_reply_id(state.azure_work_item_reply_id);
        state.azure_work_item_query = query.trim().to_string();
        state.empty_message = Some("Searching Azure DevOps work items…".to_string());
        state.results.clear();
        state.rows = vec![RowKind::Message];
        (
            state.window,
            state.azure_work_item_reply_id,
            state.azure_devops.clone(),
        )
    };
    let Some(list) = STATE.with(|state| state.borrow().list) else {
        return;
    };
    populate_empty_message(list, Some("Searching Azure DevOps work items…"));
    let Some(window) = window else {
        return;
    };
    if !settings.enabled {
        set_azure_empty_message("Azure DevOps search is disabled in Settings.");
        return;
    }
    crate::azure_devops::search_work_items_async(
        settings,
        query.trim().to_string(),
        reply_id,
        window,
        WM_QUICK_LAUNCH_AZURE_RESULTS,
    );
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
        if !accepts_azure_work_item_reply(
            state.azure_work_items_active,
            state.azure_work_item_reply_id,
            reply_id,
        ) {
            return None;
        }
        state.results = reply
            .candidates
            .into_iter()
            .map(|candidate| Entry {
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        let fetched_entries = state.results.clone();
        state.index.merge_cached_work_items(&fetched_entries);
        state.empty_message = reply.message;
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = if rows.is_empty() {
            vec![RowKind::Message]
        } else {
            rows.clone()
        };
        Some((state.list, labels, rows, state.empty_message.clone()))
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
) {
    let (window, reply_id, settings) = {
        let mut state = state.borrow_mut();
        state.azure_pull_requests_live_active = true;
        state.azure_pull_request_reply_id = next_azure_reply_id(state.azure_pull_request_reply_id);
        state.empty_message = Some("Searching Azure DevOps pull requests…".to_string());
        state.results.clear();
        state.rows = vec![RowKind::Message];
        (
            state.window,
            state.azure_pull_request_reply_id,
            state.azure_devops.clone(),
        )
    };
    let Some(list) = STATE.with(|state| state.borrow().list) else {
        return;
    };
    populate_empty_message(list, Some("Searching Azure DevOps pull requests…"));
    let Some(window) = window else {
        return;
    };
    if !settings.enabled {
        set_azure_empty_message("Azure DevOps search is disabled in Settings.");
        return;
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
        if !accepts_azure_pull_request_reply(
            state.azure_pull_requests_live_active,
            state.azure_pull_request_reply_id,
            reply_id,
        ) {
            return None;
        }
        state.results = reply
            .candidates
            .into_iter()
            .map(|candidate| Entry {
                name: candidate.name,
                breadcrumb: candidate.detail,
                path: candidate.url.clone(),
                action: crate::quick_launch::Action::OpenUrl(candidate.url),
                branch: None,
            })
            .collect();
        state.empty_message = reply.message;
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = if rows.is_empty() {
            vec![RowKind::Message]
        } else {
            rows.clone()
        };
        Some((state.list, labels, rows, state.empty_message.clone()))
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

/// 前回の検索結果だけを対象にしても漏れがない場合の、今回の検索語を返す。
///
/// 通常検索と `b ` / `h ` / `w ` / `a ` の同一モードでは、入力末尾への文字追加で
/// 一致集合が広がらない。Everything と Azure DevOps は別経路なので対象外にする。
pub(super) fn refined_search_term<'a>(previous: &str, current: &'a str) -> Option<&'a str> {
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
    } else {
        Some(("normal", query))
    }
}

/// リストボックスの中身を丸ごと差し替える。通常検索と Everything の
/// 非同期結果受信 (`handle_everything_results`) の双方から使う。
/// `rows` が空でも構わない (見出しも項目もない = 空一覧)。
/// 初期カーソルは、見出し行を飛ばした最初の項目行に置く。
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
