//! 検索窓 (上部の帯) のバッジ状態と再描画。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use super::{EDIT_HEIGHT, PADDING, State};

/// 検索窓に出すモードバッジを入力文字列から判定し、変わっていれば
/// 検索窓部分だけ再描画する。
pub(super) fn update_badge(state: &RefCell<State>, query: &str) {
    let badge = crate::quick_launch::prefix_badge(query);
    let live_hint = crate::quick_launch::azure_live_request(query).is_some();
    let label = crate::quick_launch::azure_badge_label(query);
    let (window, dpi, changed, width_changed) = {
        let mut state = state.borrow_mut();
        let changed = state.badge != badge
            || state.live_search_hint != live_hint
            || state.badge_label != label;
        // 入力欄の幅はラベルの長さで決まる。変わったときだけ配置し直す
        let width_changed = super::layout::badge_slot_width(label.as_deref(), state.dpi)
            != super::layout::badge_slot_width(state.badge_label.as_deref(), state.dpi);
        state.badge = badge;
        state.live_search_hint = live_hint;
        state.badge_label = label;
        (state.window, state.dpi, changed, width_changed)
    };
    if width_changed {
        super::layout::relayout_edit();
    }
    if changed {
        invalidate_search_bar(window, dpi);
        // ヒント帯の `Ctrl+Enter` はバッジと同じ条件で出し入れする
        super::draw_footer::invalidate_footer();
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
