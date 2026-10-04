//! WndProc 本体とメッセージハンドラ。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{FillRect, HDC, SetBkColor, SetTextColor};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, EN_CHANGE, GetClientRect, LBN_DBLCLK, LBN_SELCHANGE, MoveWindow, WM_ACTIVATE,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_DRAWITEM, WM_ERASEBKGND,
    WM_MEASUREITEM, WM_PAINT, WM_SETFOCUS, WM_SIZE,
};

use super::azure_live::{
    handle_azure_pipeline_results, handle_azure_pull_request_results,
    handle_azure_work_item_results,
};
use super::draw::paint_window;
use super::draw_row::draw_list_item;
use super::input::{hide_window, queue_selected};
use super::layout::{edit_width, scale};
use super::search::{handle_everything_results, update_results};
use super::{
    BACKGROUND, EDIT_HEIGHT, FOOTER_GAP, FOOTER_HEIGHT, HEADER_HEIGHT, PADDING, ROW_HEIGHT,
    RowKind, SEARCH_ICON_WIDTH, STATE, SURFACE, TEXT_PRIMARY, WM_QUICK_LAUNCH_AZURE_NOTICE,
    WM_QUICK_LAUNCH_AZURE_RESULTS,
};

pub(super) fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_SETFOCUS => {
            // 遅れて届く WM_ACTIVATE の既定処理は親へフォーカスを置く。
            // show() で設定済みでも、入力先を検索欄へ戻す必要がある。
            let edit = STATE.with(|state| state.borrow().edit);
            if let Some(edit) = edit {
                unsafe {
                    let _ = SetFocus(Some(edit));
                }
            }
            LRESULT(0)
        }
        crate::icon::WM_ICON_READY => {
            crate::icon::apply_ready();
            let list = STATE.with(|state| state.borrow().list);
            if let Some(list) = list {
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(list), None, false);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            let control = HWND(lparam.0 as *mut _);
            let is_edit = STATE.with(|state| Some(control) == state.borrow().edit);
            let is_list = STATE.with(|state| Some(control) == state.borrow().list);
            if is_edit && notification == EN_CHANGE {
                STATE.with(update_results);
            } else if is_list && notification == LBN_DBLCLK {
                queue_selected();
            } else if is_list && notification == LBN_SELCHANGE {
                // マウスでの選択変更。ヒント帯を選択中の候補に合わせる
                super::draw_footer::invalidate_footer();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            // MoveWindow は WM_ERASEBKGND / WM_PAINT を同期送信して
            // window_proc を再入させる。借用を解放してから呼ぶ
            let (edit, list, dpi, badge_label) = STATE.with(|state| {
                let state = state.borrow();
                (state.edit, state.list, state.dpi, state.badge_label.clone())
            });
            let padding = scale(PADDING, dpi);
            let edit_height = scale(EDIT_HEIGHT, dpi);
            let icon_width = scale(SEARCH_ICON_WIDTH, dpi);
            // 入力欄は 1 行の EDIT で、文字は上端から描かれる。16px の文字に
            // 合わせた高さの箱を検索窓の縦中央に置いて、文字を中央に揃える
            let input_height = scale(24, dpi);
            let footer = scale(FOOTER_GAP + FOOTER_HEIGHT, dpi);
            unsafe {
                if let Some(edit) = edit {
                    let _ = MoveWindow(
                        edit,
                        padding + icon_width,
                        padding + (edit_height - input_height) / 2,
                        edit_width(width, dpi, badge_label.as_deref()),
                        input_height,
                        true,
                    );
                }
                if let Some(list) = list {
                    let top = padding + edit_height + scale(6, dpi);
                    let _ = MoveWindow(
                        list,
                        padding,
                        top,
                        width - padding * 2,
                        height - top - footer,
                        true,
                    );
                }
            }
            LRESULT(0)
        }
        WM_DRAWITEM => {
            if lparam.0 != 0 {
                unsafe { draw_list_item(&*(lparam.0 as *const DRAWITEMSTRUCT)) };
            }
            LRESULT(1)
        }
        WM_MEASUREITEM => {
            if lparam.0 != 0 {
                unsafe { measure_list_item(&mut *(lparam.0 as *mut MEASUREITEMSTRUCT)) };
            }
            LRESULT(1)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as *mut _);
            STATE.with(|state| {
                let state = state.borrow();
                unsafe {
                    SetTextColor(hdc, TEXT_PRIMARY);
                    SetBkColor(hdc, SURFACE);
                }
                LRESULT(state.surface_brush.map_or(0, |brush| brush.0 as isize))
            })
        }
        WM_CTLCOLORLISTBOX => {
            let hdc = HDC(wparam.0 as *mut _);
            STATE.with(|state| {
                let state = state.borrow();
                unsafe {
                    SetTextColor(hdc, TEXT_PRIMARY);
                    SetBkColor(hdc, BACKGROUND);
                }
                LRESULT(state.background_brush.map_or(0, |brush| brush.0 as isize))
            })
        }
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut _);
            let mut rect = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut rect);
            }
            STATE.with(|state| {
                if let Some(brush) = state.borrow().background_brush {
                    unsafe {
                        FillRect(hdc, &rect, brush);
                    }
                }
            });
            LRESULT(1)
        }
        WM_PAINT => {
            paint_window(hwnd);
            LRESULT(0)
        }
        WM_ACTIVATE if (wparam.0 & 0xffff) == 0 => {
            let owner = STATE.with(|state| state.borrow().owner);
            hide_window(Some(hwnd), owner);
            LRESULT(0)
        }
        WM_CLOSE => {
            let owner = STATE.with(|state| state.borrow().owner);
            hide_window(Some(hwnd), owner);
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_AZURE_RESULTS => {
            // Work Item / PR / Pipeline のライブ検索は reply_id の名前空間が
            // それぞれ別 (State::azure_work_item_reply_id /
            // azure_pull_request_reply_id / azure_pipeline_reply_id)。
            // どの要求への応答かはハンドラ内の take_*_results が判定するので、
            // 3 つとも呼んでも無関係な分は None で素通りする。
            handle_azure_work_item_results(wparam.0 as u32);
            handle_azure_pull_request_results(wparam.0 as u32);
            handle_azure_pipeline_results(wparam.0 as u32);
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_AZURE_NOTICE => {
            super::azure_notice::handle_loaded();
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA => {
            if lparam.0 != 0 {
                unsafe {
                    let copy_data =
                        &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                    let reply_id = copy_data.dwData as u32;
                    let is_current = STATE.with(|state| {
                        let state = state.borrow();
                        super::search::accepts_everything_reply(
                            state.everything_active,
                            state.everything_reply_id,
                            reply_id,
                        )
                    });
                    if is_current
                        && !copy_data.lpData.is_null()
                        && copy_data.cbData > 0
                        && copy_data.cbData as usize <= crate::everything::MAX_RESULT_BYTES
                    {
                        // Everything はこのハンドラから戻ると lpData を解放する。
                        // 保持するならここでコピーする必要がある (SDK の注記通り)
                        let bytes = std::slice::from_raw_parts(
                            copy_data.lpData.cast::<u8>(),
                            copy_data.cbData as usize,
                        )
                        .to_vec();
                        handle_everything_results(reply_id, &bytes);
                    }
                }
            }
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// セクション見出し行 (`RowKind::Header`) は通常項目より低く測る。
/// リストボックスは `itemData` を持たないため `itemID` で `state.rows` を引く。
unsafe fn measure_list_item(measure: &mut MEASUREITEMSTRUCT) {
    let (row, dpi) = STATE.with(|state| {
        let state = state.borrow();
        (state.rows.get(measure.itemID as usize).copied(), state.dpi)
    });
    measure.itemHeight = match row {
        Some(RowKind::Header(_)) => scale(HEADER_HEIGHT, dpi) as u32,
        _ => scale(ROW_HEIGHT, dpi) as u32,
    };
}
