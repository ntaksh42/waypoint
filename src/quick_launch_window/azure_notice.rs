//! 直近の Azure DevOps 同期が失敗していることを、`az` 一覧の先頭へ知らせる (FR-9.18.7)。
//!
//! 失敗は SQLite (`project_state.last_error`) とログにしか残らず、`az` の一覧は
//! 失敗しても空や古いままに見えた。読み取りは SQLite を開くため、表示経路
//! (キー入力) では行わない。`az ` に入った最初の入力でバックグラウンドへ依頼し、
//! 結果をメッセージで受けて一覧を組み直す。

use std::cell::RefCell;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::HSTRING;

use super::{RowKind, STATE, State, WM_QUICK_LAUNCH_AZURE_NOTICE};
use crate::azure_devops::CacheStatus;

/// 取得スレッドから UI スレッドへの受け渡し。`Some(None)` は「取得済みで失敗なし」。
static PENDING: Mutex<Option<Option<String>>> = Mutex::new(None);

/// 通知に載せる失敗理由の最大文字数。1 行に収める。
const MAX_REASON_CHARS: usize = 90;

/// 同期状態から通知文を作る。失敗が無い、または同期中 (状態が揺れている) なら `None`。
pub(super) fn notice_text(status: &CacheStatus) -> Option<String> {
    if status.refresh_in_progress || status.failed_projects == 0 {
        return None;
    }
    let reason = status.last_error.as_deref().unwrap_or("see log").trim();
    let reason: String = if reason.chars().count() > MAX_REASON_CHARS {
        reason
            .chars()
            .take(MAX_REASON_CHARS)
            .chain(std::iter::once('\u{2026}'))
            .collect()
    } else {
        reason.to_string()
    };
    // 401 / 403 は認証の問題。理由の文面だけでは直し方が分からないので添える
    let hint = if reason.contains("HTTP 401") || reason.contains("HTTP 403") {
        " (check the PAT in Settings or run `az login`)"
    } else {
        ""
    };
    Some(format!(
        "! {} failed to sync: {reason}{hint}",
        if status.failed_projects == 1 {
            "1 project".to_string()
        } else {
            format!("{} projects", status.failed_projects)
        }
    ))
}

/// この表示中にまだ依頼していなければ、同期状態の取得をバックグラウンドへ依頼する。
/// 設定で無効、または監視対象が無いときは何もしない。
pub(super) fn request(state: &RefCell<State>) {
    let (settings, window) = {
        let mut state = state.borrow_mut();
        if state.azure_notice_requested
            || !state.azure_devops.enabled
            || state.azure_devops.projects.is_empty()
        {
            return;
        }
        state.azure_notice_requested = true;
        let Some(window) = state.window else {
            return;
        };
        (state.azure_devops.clone(), window.0 as isize)
    };
    std::thread::spawn(move || {
        let text = notice_text(&crate::azure_devops::cache_status(&settings));
        if let Ok(mut pending) = PENDING.lock() {
            *pending = Some(text);
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(window as *mut _)),
                WM_QUICK_LAUNCH_AZURE_NOTICE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}

/// `WM_QUICK_LAUNCH_AZURE_NOTICE`: 取得結果を反映する。通知が出る状態に変わり、
/// なお `az` の入力中なら一覧を組み直す (失敗なしなら表示は変わらないので何もしない)。
pub(super) fn handle_loaded() {
    let Some(text) = PENDING.lock().ok().and_then(|mut pending| pending.take()) else {
        return;
    };
    let changed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let changed = state.azure_notice != text;
        state.azure_notice = text;
        changed
    });
    if !changed {
        return;
    }
    let edit = STATE.with(|state| state.borrow().edit);
    let query = edit.map(super::input::read_text).unwrap_or_default();
    if query.starts_with(crate::quick_launch::AZURE_DEVOPS_PREFIX) {
        STATE.with(super::search::update_results);
    }
}

/// 一覧の先頭に通知行を足す。通知が無い、または行が 1 つも無いときは何もしない。
/// 説明文 `Message` だけの一覧にも足す (0 件の原因が同期失敗かもしれないため)。
pub(super) fn prepend(state: &State, labels: &mut Vec<HSTRING>, rows: &mut Vec<RowKind>) {
    let Some(notice) = &state.azure_notice else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    labels.insert(0, HSTRING::from(notice.as_str()));
    rows.insert(0, RowKind::Notice);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(failed: usize, error: Option<&str>, refreshing: bool) -> CacheStatus {
        CacheStatus {
            refreshed_at: Some(1),
            failed_projects: failed,
            last_error: error.map(str::to_string),
            refresh_in_progress: refreshing,
        }
    }

    #[test]
    fn no_notice_without_failures_or_during_a_refresh() {
        assert_eq!(notice_text(&status(0, None, false)), None);
        assert_eq!(notice_text(&status(1, Some("x"), true)), None);
    }

    #[test]
    fn notice_names_the_count_and_the_reason() {
        assert_eq!(
            notice_text(&status(1, Some("No PAT is saved."), false)).as_deref(),
            Some("! 1 project failed to sync: No PAT is saved.")
        );
        assert_eq!(
            notice_text(&status(3, Some("HTTP 500"), false)).as_deref(),
            Some("! 3 projects failed to sync: HTTP 500")
        );
        assert_eq!(
            notice_text(&status(2, None, false)).as_deref(),
            Some("! 2 projects failed to sync: see log")
        );
    }

    #[test]
    fn auth_failures_point_at_the_fix() {
        let text = notice_text(&status(
            2,
            Some("Azure DevOps request returned HTTP 401 Unauthorized"),
            false,
        ))
        .unwrap();
        assert!(
            text.ends_with("(check the PAT in Settings or run `az login`)"),
            "{text}"
        );
    }

    #[test]
    fn long_reasons_are_cut_to_one_line() {
        let text = notice_text(&status(1, Some(&"e".repeat(300)), false)).unwrap();
        assert!(text.chars().count() < 140, "{text}");
        assert!(text.ends_with('\u{2026}'));
    }

    #[test]
    fn prepend_puts_the_notice_first_but_not_on_an_empty_list() {
        let state = State {
            azure_notice: Some("! 1 project failed to sync: x".to_string()),
            ..State::default()
        };
        let mut labels = Vec::new();
        let mut rows = Vec::new();
        prepend(&state, &mut labels, &mut rows);
        assert!(rows.is_empty());

        // 説明文だけの一覧にも通知は付く
        let mut labels = vec![HSTRING::from("No matches")];
        let mut rows = vec![RowKind::Message];
        prepend(&state, &mut labels, &mut rows);
        assert!(matches!(rows[..], [RowKind::Notice, RowKind::Message]));

        let mut labels = vec![HSTRING::from("a")];
        let mut rows = vec![RowKind::Item(0)];
        prepend(&state, &mut labels, &mut rows);
        assert!(matches!(rows[0], RowKind::Notice));
        assert!(matches!(rows[1], RowKind::Item(0)));
        assert_eq!(labels.len(), 2);
    }
}
