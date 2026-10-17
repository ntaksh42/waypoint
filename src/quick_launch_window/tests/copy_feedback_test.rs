use std::cell::RefCell;

use super::super::{State, search::update_results};
use crate::quick_launch::Snapshot;
use windows::Win32::Graphics::Gdi::{GetUpdateRect, ValidateRect};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowTextW, WS_EX_NOACTIVATE, WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

#[test]
fn editing_any_search_mode_clears_copied_and_repaints_the_badge() {
    unsafe {
        // フォーカスを奪わない画面外の編集欄で、入力から再描画要求まで確認する。
        let edit = CreateWindowExW(
            WS_EX_NOACTIVATE,
            w!("EDIT"),
            w!(""),
            WS_POPUP | WS_VISIBLE,
            -32000,
            -32000,
            400,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let state = RefCell::new(State {
            edit: Some(edit),
            window: Some(edit),
            dpi: 96,
            process_snapshot: Some(Snapshot::new(Vec::new())),
            ..State::default()
        });
        for query in [
            "folder",
            "f ",
            "k ",
            "vs ",
            "cs ",
            "az wit ",
            "az optimize ",
            "az ",
        ] {
            // モード変更時と、同じモードで続けて入力した場合の両方を確認する。
            for query in [query.to_string(), format!("{query}x")] {
                SetWindowTextW(edit, &HSTRING::from(&query)).unwrap();
                let _ = ValidateRect(Some(edit), None);
                state.borrow_mut().copy_feedback = true;

                update_results(&state);

                assert!(!state.borrow().copy_feedback, "{query}");
                assert_eq!(
                    state.borrow().badge,
                    crate::quick_launch::prefix_badge(&query)
                );
                assert!(GetUpdateRect(edit, None, false).as_bool(), "{query}");
            }
        }
        DestroyWindow(edit).unwrap();
    }
}
