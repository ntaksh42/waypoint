//! テキスト編集・リスト選択・実行系の操作。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, GetWindowTextW, KillTimer, LB_GETCURSEL, LB_SETCURSEL, PostMessageW,
    SW_HIDE, SetWindowTextW, ShowWindow,
};
use windows::core::{HSTRING, w};

use super::search::invalidate_search_bar;
use super::{LIVE_SEARCH_TIMER_ID, RowKind, STATE, WM_QUICK_LAUNCH_EXECUTE};
use crate::config::OpenMode;
use crate::quick_launch::{Action, Entry};

pub(super) fn read_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        let mut text = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut text);
        String::from_utf16_lossy(&text[..copied as usize])
    }
}

/// カーソル直前の単語の開始位置を返す。
/// 「単語」は空白の連続とそれ以外の連続の境界で区切る単純な定義。
pub(super) fn word_start_before(text: &[u16], cursor: usize) -> usize {
    let mut start = cursor;
    while start > 0 && text[start - 1] == b' ' as u16 {
        start -= 1;
    }
    while start > 0 && text[start - 1] != b' ' as u16 {
        start -= 1;
    }
    start
}

/// Ctrl+Backspace: カーソル直前の単語を削除する。
pub(super) fn delete_word_before_cursor(edit: HWND) {
    let text: Vec<u16> = read_text(edit).encode_utf16().collect();
    let mut sel_start = 0u32;
    let mut sel_end = 0u32;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_GETSEL,
            Some(WPARAM(&mut sel_start as *mut u32 as usize)),
            Some(LPARAM(&mut sel_end as *mut u32 as isize)),
        );
    }
    let cursor = sel_start.min(sel_end) as usize;
    if cursor == 0 {
        return;
    }
    let start = word_start_before(&text, cursor);
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(start)),
            Some(LPARAM(cursor as isize)),
        );
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(w!("").as_ptr() as isize)),
        );
    }
}

/// リストボックスの選択位置を読む。
///
/// `SendMessageW` を呼ぶため、STATE を借用したまま渡さないこと。
/// 引数はハンドルだけを受け取る。
pub(super) fn current_selection(list: Option<HWND>) -> Option<usize> {
    let list = list?;
    let selected = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(list, LB_GETCURSEL, None, None).0
    };
    (selected >= 0).then_some(selected as usize)
}

pub(super) fn select_at(list: Option<HWND>, index: usize) {
    let Some(list) = list else {
        return;
    };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_SETCURSEL,
            Some(WPARAM(index)),
            None,
        );
    }
}

/// 選択を相対移動する。借用は最初に済ませ、以降は Win32 のみ触る。
pub(super) fn move_selection(delta: isize) {
    let (list, rows) = STATE.with(|state| {
        let state = state.borrow();
        (state.list, state.rows.clone())
    });
    let current = current_selection(list).unwrap_or(0);
    let Some(next) = next_selectable_row(&rows, current, delta) else {
        return;
    };
    select_at(list, next);
}

/// `move_selection` の索引計算だけを取り出した純粋関数。区分見出し行は
/// 選択対象外なので、着地点がそれに重なったら同じ方向へさらに進めて
/// 次の項目行を探す (見出しが連続することはない前提)。
pub(super) fn next_selectable_row(rows: &[RowKind], current: usize, delta: isize) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let step = if delta < 0 { -1isize } else { 1isize };
    let mut next = current.saturating_add_signed(delta).min(rows.len() - 1);
    while matches!(rows.get(next), Some(RowKind::Header(_))) {
        let stepped = next.checked_add_signed(step)?;
        if stepped >= rows.len() {
            return None;
        }
        next = stepped;
    }
    Some(next)
}

/// Home キー: 先頭の選択可能な項目行。先頭が見出し行なら後方向へ探す
/// (`move_selection` と違い、常に先頭からの前進で確定させる)。
pub(super) fn first_selectable_row(rows: &[RowKind]) -> Option<usize> {
    rows.iter()
        .position(|row| !matches!(row, RowKind::Header(_)))
}

/// End キー: 末尾の選択可能な項目行。末尾が見出し行なら前方向へ探す。
pub(super) fn last_selectable_row(rows: &[RowKind]) -> Option<usize> {
    rows.iter()
        .rposition(|row| !matches!(row, RowKind::Header(_)))
}

/// `state.rows[row]` が指す項目行の `Entry` を返す。見出し・メッセージ行は `None`。
pub(super) fn entry_at_row(state: &super::State, row: usize) -> Option<Entry> {
    match state.rows.get(row)? {
        RowKind::Item(index) => state.results.get(*index).cloned(),
        RowKind::Header(_) | RowKind::Message => None,
    }
}

pub(super) fn queue_selected() {
    // LB_GETCURSEL は SendMessageW。借用の外で読んでおく
    let list = STATE.with(|state| state.borrow().list);
    let selected = current_selection(list);

    enum Selected {
        Replace(HWND, String),
        Execute(Option<HWND>, Option<HWND>),
        LiveWorkItemSearch(String),
        LivePullRequestSearch(crate::quick_launch::PullRequestFilter, String),
        LivePipelineSearch(crate::quick_launch::PipelineFilter, String),
    }

    let selected = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut entry = selected.and_then(|row| entry_at_row(&state, row))?;
        if let Action::ReplaceQuery(query) = &entry.action {
            return state
                .edit
                .map(|edit| Selected::Replace(edit, query.clone()));
        }
        if let Action::AzureLiveWorkItemSearch(query) = &entry.action {
            return Some(Selected::LiveWorkItemSearch(query.clone()));
        }
        if let Action::AzureLivePullRequestSearch { filter, query } = &entry.action {
            return Some(Selected::LivePullRequestSearch(*filter, query.clone()));
        }
        if let Action::AzureLivePipelineSearch { filter, query } = &entry.action {
            return Some(Selected::LivePipelineSearch(*filter, query.clone()));
        }
        if let Action::OpenFolder(mode) = &mut entry.action {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            if shift {
                *mode = OpenMode::NewWindow;
            } else if control {
                *mode = OpenMode::Reuse;
            }
        }
        state.pending = Some(entry);
        Some(Selected::Execute(state.window, state.owner))
    });
    match selected {
        Some(Selected::Replace(edit, query)) => unsafe {
            let cursor = query.encode_utf16().count();
            let _ = SetWindowTextW(edit, &HSTRING::from(query));
            let _ = SetFocus(Some(edit));
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                EM_SETSEL,
                Some(WPARAM(cursor)),
                Some(LPARAM(cursor as isize)),
            );
        },
        Some(Selected::Execute(window, owner)) => {
            hide_window(window);
            if let Some(owner) = owner {
                unsafe {
                    let _ =
                        PostMessageW(Some(owner), WM_QUICK_LAUNCH_EXECUTE, WPARAM(0), LPARAM(0));
                }
            }
        }
        Some(Selected::LiveWorkItemSearch(query)) => {
            STATE.with(|state| super::search::start_azure_work_item_live_search(state, &query));
        }
        Some(Selected::LivePullRequestSearch(filter, query)) => {
            STATE.with(|state| {
                super::search::start_azure_pull_request_live_search(state, filter, &query)
            });
        }
        Some(Selected::LivePipelineSearch(filter, query)) => {
            STATE.with(|state| {
                super::search::start_azure_pipeline_live_search(state, filter, &query)
            });
        }
        None => {}
    }
}

/// 現在選択中の候補を複製して返す。`SendMessageW` を挟むため、
/// 呼び出し側は返り値を得てから STATE を再度借用すること。
pub(super) fn selected_entry() -> Option<Entry> {
    let list = STATE.with(|state| state.borrow().list);
    let selected = current_selection(list)?;
    STATE.with(|state| entry_at_row(&state.borrow(), selected))
}

/// `Ctrl+Shift+Enter`: 選択中の候補を config へ登録するよう常駐部へ
/// 依頼する。ウィンドウは閉じず、続けて他の候補も登録できるようにする。
pub(super) fn add_selected_to_favorites() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.to_item().is_none() {
        // ウィンドウ / URL 候補は登録対象外。無音で無視する
        // (FocusWindow・OpenUrl は config へ永続化できないため)
        return;
    }
    let owner = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.pending_add = Some(entry);
        state.owner
    });
    if let Some(owner) = owner {
        unsafe {
            let _ = PostMessageW(
                Some(owner),
                super::WM_QUICK_LAUNCH_ADD_TO_FAVORITES,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// `Ctrl+C`: 選択中候補のパスをクリップボードへコピーする。
pub(super) fn copy_selected_path() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.path.is_empty() || crate::clipboard::set_text(&entry.path).is_err() {
        return;
    }
    STATE.with(|state| {
        let (window, dpi) = {
            let mut state = state.borrow_mut();
            state.copy_feedback = true;
            (state.window, state.dpi)
        };
        invalidate_search_bar(window, dpi);
    });
}

/// `Ctrl+E`: 選択中候補をエクスプローラーで開き、対象を選択状態にする。
pub(super) fn reveal_selected_in_explorer() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.path.is_empty() {
        return;
    }
    let window = STATE.with(|state| state.borrow().window);
    hide_window(window);
    let _ = crate::shell::reveal_in_explorer(&entry.path);
}

pub(super) fn hide_window(window: Option<HWND>) {
    if let Some(window) = window {
        unsafe {
            let _ = KillTimer(Some(window), LIVE_SEARCH_TIMER_ID);
            let _ = ShowWindow(window, SW_HIDE);
        }
    }
    STATE.with(|state| state.borrow_mut().pending_live_search = None);
}
