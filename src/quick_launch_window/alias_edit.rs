//! `F2` で候補に別名を付ける入力モード (FR-9.30)。
//!
//! 専用のダイアログは作らず、検索欄をそのまま別名の入力欄として使う。
//! モード中は一覧に対象の候補だけを出し、`Enter` で保存、`Esc` で
//! 元の入力へ戻る。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindowVisible, KillTimer, SendMessageW, SetTimer, SetWindowTextW,
};
use windows::core::HSTRING;

use super::list::{build_rows, populate_list, with_message};
use super::{STATE, State};
use crate::quick_launch::Entry;
use crate::quick_launch_aliases::{Aliases, LiveAliasTarget, parse};

pub(super) const LIVE_ALIAS_TIMER: usize = 1;

/// 別名を検索中だけ監視し、別アプリが対象を閉じた場合も表示から外す。
pub(super) fn watch_live_results(state: &RefCell<State>, query: &str) {
    if query.is_empty() {
        return;
    }
    let window = {
        let state = state.borrow();
        state
            .window
            .filter(|_| state.index.aliases.live.matches_query(query))
    };
    if let Some(window) = window {
        unsafe { SetTimer(Some(window), LIVE_ALIAS_TIMER, 250, None) };
    }
}

pub(super) fn check_live_results(window: HWND) {
    STATE.with(|state| {
        let (changed, watching) = {
            let mut state = state.borrow_mut();
            let watching = unsafe { IsWindowVisible(window).as_bool() }
                && state
                    .previous_query
                    .as_ref()
                    .is_some_and(|query| state.index.aliases.live.matches_query(query));
            let changed = watching && state.index.prune_live_aliases();
            if changed {
                state.previous_query = None;
            }
            (changed, watching)
        };
        if !watching || changed {
            unsafe {
                let _ = KillTimer(Some(window), LIVE_ALIAS_TIMER);
            }
        }
        if changed {
            refresh_alias_results(state);
        }
    });
}

/// 対象が更新されたときも、残っている行なら選択を維持する。
pub(super) fn refresh_alias_results(state: &RefCell<State>) {
    let selected = super::input::selected_entry();
    super::search::update_results(state);
    let Some(selected) = selected else {
        return;
    };
    let (list, row) = {
        let state = state.borrow();
        let row = state.rows.iter().position(|row| {
            let super::RowKind::Item(index) = *row else {
                return false;
            };
            let Some(entry) = state.results.get(index) else {
                return false;
            };
            use crate::quick_launch::Action::FocusBrowserTab;
            match (&selected.action, &entry.action) {
                (FocusBrowserTab(old), FocusBrowserTab(new)) => {
                    old.source == new.source
                        && old.browser == new.browser
                        && old.tab_id == new.tab_id
                }
                _ => selected == *entry,
            }
        });
        (state.list, row)
    };
    if let Some(row) = row {
        super::input::select_at(list, row);
    }
}

/// 入力モードの状態。
pub(in crate::quick_launch_window) struct AliasEdit {
    target: Entry,
    live: Result<Option<LiveAliasTarget>, String>,
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
        let live = state.index.aliases.live.begin(&target);
        let current = match &live {
            Ok(Some(target)) => state.index.aliases.live.names(target.key()).join(" "),
            Ok(None) => state.index.aliases.aliases_of(&target).join(" "),
            Err(_) => String::new(),
        };
        let saved_query = super::input::read_text(edit);
        state.alias_edit = Some(AliasEdit {
            target,
            error: live.as_ref().err().cloned(),
            live,
            saved_query,
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
        let result = parse(&text).and_then(|aliases| match &editing.live {
            Ok(Some(target)) => {
                if state.index.live_alias_entry(target.key()).is_none() {
                    return Err("This window or tab is no longer open".to_owned());
                }
                state.index.aliases.live.set(target.clone(), aliases);
                Ok(())
            }
            Ok(None) => {
                state.index.aliases.set(&editing.target, aliases);
                state
                    .index
                    .aliases
                    .save()
                    .map_err(|error| format!("Could not save aliases: {error}"))
            }
            Err(error) => Err(error.clone()),
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
            let label = if matches!(editing.live, Ok(Some(_))) {
                "Temporary aliases"
            } else {
                "Aliases"
            };
            format!(
                "{label} for \"{}\" (space separated). Enter: save, Esc: cancel. Empty removes.",
                editing.target.name
            )
        });
        let results = vec![editing.target.clone()];
        let (labels, rows) = build_rows(&results, &[]);
        let (labels, rows) = with_message(labels, rows, Some(&message));
        state.results = results;
        state.rows = rows.clone();
        state.empty_message = Some(message);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_tabs::{Browser, Tab};
    use crate::window_instance::WindowMarker;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};
    use windows::core::w;

    #[test]
    fn tab_alias_survives_another_profile_update_and_index_install() {
        let host = || unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Tab host"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let (first, second) = (host(), host());
        let first_marker = WindowMarker::attach(first).unwrap();
        let second_marker = WindowMarker::attach(second).unwrap();
        let tab = |title: &str| Tab {
            id: 1,
            window_id: 1,
            title: title.into(),
            url: "https://example.test/".into(),
            source: None,
        };
        let replace = crate::quick_launch_window::replace_browser_tabs;
        replace(
            Browser::Edge,
            Some(first_marker.0),
            vec![tab("First profile")],
        );
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let index = &mut state.index;
            let target = index.aliases.live.begin(&index.tabs[0]).unwrap().unwrap();
            index.aliases.live.set(target, vec!["aliasprobe".into()]);
        });
        replace(
            Browser::Edge,
            Some(second_marker.0),
            vec![tab("Another profile")],
        );
        replace(Browser::Edge, Some(second_marker.0), Vec::new());
        super::super::configure::install_index(
            Default::default(),
            &Default::default(),
            &Default::default(),
        );
        STATE.with(|state| {
            let matches = state
                .borrow()
                .index
                .apply_aliases("aliasprobe", Vec::new(), 10);
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].name, "First profile");
        });
        replace(Browser::Edge, Some(first_marker.0), Vec::new());
        assert!(STATE.with(|state| state.borrow().index.aliases.live.is_empty()));
        unsafe {
            DestroyWindow(first).unwrap();
            DestroyWindow(second).unwrap();
        }
    }
}
