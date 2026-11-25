//! `F2` で候補に別名を付ける入力モード (FR-9.30)。
//!
//! 専用のダイアログは作らず、検索欄をそのまま別名の入力欄として使う。
//! モード中は一覧に対象の候補だけを出し、`Enter` で保存、`Esc` で
//! 元の入力へ戻る。

use std::cell::RefCell;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, SetWindowTextW};
use windows::core::HSTRING;

use super::list::{build_rows, populate_list, with_message};
use super::{STATE, State};
use crate::quick_launch::Entry;
use crate::quick_launch_aliases::{Aliases, parse};

/// 入力モードの状態。
pub(in crate::quick_launch_window) struct AliasEdit {
    target: Entry,
    /// `F2` を押す前の検索語。保存・取り消しの後に戻す。
    saved_query: String,
    error: Option<String>,
}

pub(super) fn is_active() -> bool {
    STATE.with(|state| state.borrow().alias_edit.is_some())
}

/// `F2`: 選択中の候補の別名入力を始める。付けられない候補なら何もしない。
pub(super) fn begin() {
    if is_active() {
        return;
    }
    let Some(target) = super::input::selected_entry() else {
        return;
    };
    if !Aliases::supports(&target) {
        return;
    }
    let edit = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let edit = state.edit?;
        let current = state.index.aliases.aliases_of(&target).join(" ");
        let saved_query = super::input::read_text(edit);
        state.alias_edit = Some(AliasEdit {
            target,
            saved_query,
            error: None,
        });
        Some((edit, current))
    });
    let Some((edit, current)) = edit else {
        return;
    };
    // 借用の外で書き換える。EN_CHANGE が同期で届き、`render` が走る
    set_text(edit, &current);
    STATE.with(render);
}

/// `Enter`: 入力を保存して元の検索へ戻る。使えない別名があればモードに留まる。
pub(super) fn commit() {
    let Some(edit) = STATE.with(|state| state.borrow().edit) else {
        return;
    };
    let text = super::input::read_text(edit);
    let saved = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = &mut *state;
        let editing = state.alias_edit.as_mut()?;
        let result = parse(&text).and_then(|aliases| {
            state.index.aliases.set(&editing.target, aliases);
            state
                .index
                .aliases
                .save()
                .map_err(|error| format!("Could not save aliases: {error}"))
        });
        match result {
            Ok(()) => Some(true),
            Err(message) => {
                editing.error = Some(message);
                Some(false)
            }
        }
    });
    match saved {
        Some(true) => finish(),
        Some(false) => {
            STATE.with(render);
        }
        None => {}
    }
}

/// `Esc`: 保存せずに元の検索へ戻る。
pub(super) fn cancel() {
    finish();
}

/// モードを抜け、`F2` 前の検索語へ戻して一覧を作り直す。
fn finish() {
    let restored = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let editing = state.alias_edit.take()?;
        state.previous_query = None;
        Some((state.edit?, editing.saved_query))
    });
    let Some((edit, query)) = restored else {
        return;
    };
    set_text(edit, &query);
    // 戻す文字列が入力中と同じだと EN_CHANGE が来ないことがあるので明示的に組み直す
    STATE.with(super::search::update_results);
}

/// モード中の一覧: 説明 (またはエラー) の行と、対象の候補 1 行。
/// モード中なら `true` を返し、通常の検索を止める。
pub(super) fn render(state: &RefCell<State>) -> bool {
    let (list, labels, rows) = {
        let mut state = state.borrow_mut();
        let Some(editing) = &state.alias_edit else {
            return false;
        };
        let message = editing.error.clone().unwrap_or_else(|| {
            format!(
                "Aliases for \"{}\" (space separated). Enter: save, Esc: cancel. Empty removes.",
                editing.target.name
            )
        });
        let results = vec![editing.target.clone()];
        let (labels, rows) = build_rows(&results, &[]);
        let (labels, rows) = with_message(labels, rows, Some(&message));
        state.results = results;
        state.rows = rows.clone();
        state.empty_message = None;
        state.highlight_term = String::new();
        state.badge = Some("ALIAS");
        (state.list, labels, rows)
    };
    if let Some(list) = list {
        populate_list(list, &labels, &rows);
    }
    let (window, dpi) = {
        let state = state.borrow();
        (state.window, state.dpi)
    };
    super::search::invalidate_search_bar(window, dpi);
    true
}

fn set_text(edit: windows::Win32::Foundation::HWND, text: &str) {
    let cursor = text.encode_utf16().count();
    unsafe {
        let _ = SetWindowTextW(edit, &HSTRING::from(text));
        SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(cursor)),
            Some(LPARAM(cursor as isize)),
        );
    }
}
