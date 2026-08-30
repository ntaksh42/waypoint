//! WndProc 本体とメッセージハンドラ。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{FillRect, HDC, SetBkColor, SetTextColor};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, EN_CHANGE, GetClientRect, LBN_DBLCLK, MoveWindow, WM_ACTIVATE, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_DRAWITEM, WM_ERASEBKGND, WM_MEASUREITEM,
    WM_PAINT, WM_SIZE,
};

use super::draw::{draw_list_item, paint_window};
use super::input::{hide_window, queue_selected};
use super::layout::scale;
use super::search::{
    handle_azure_pipeline_results, handle_azure_pull_request_results,
    handle_azure_work_item_results, handle_everything_results, update_results,
};
use super::{
    BACKGROUND, BADGE_WIDTH, EDIT_HEIGHT, HEADER_HEIGHT, PADDING, ROW_HEIGHT, RowKind, STATE,
    SURFACE, TEXT_PRIMARY, WM_QUICK_LAUNCH_AZURE_RESULTS,
};

pub(super) fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            let control = HWND(lparam.0 as *mut _);
            let is_edit = STATE.with(|state| Some(control) == state.borrow().edit);
            let is_list = STATE.with(|state| Some(control) == state.borrow().list);
            if is_edit && notification == EN_CHANGE {
                STATE.with(update_results);
            } else if is_list && notification == LBN_DBLCLK {
                queue_selected();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            // MoveWindow は WM_ERASEBKGND / WM_PAINT を同期送信して
            // window_proc を再入させる。借用を解放してから呼ぶ
            let (edit, list, dpi) = STATE.with(|state| {
                let state = state.borrow();
                (state.edit, state.list, state.dpi)
            });
            let padding = scale(PADDING, dpi);
            let edit_height = scale(EDIT_HEIGHT, dpi);
            let badge_width = scale(BADGE_WIDTH, dpi);
            unsafe {
                if let Some(edit) = edit {
                    let _ = MoveWindow(
                        edit,
                        padding + scale(6, dpi),
                        padding + scale(6, dpi),
                        width - padding * 2 - badge_width - scale(14, dpi),
                        edit_height - scale(12, dpi),
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
                        height - top - padding,
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
            hide_window(Some(hwnd));
            LRESULT(0)
        }
        WM_CLOSE => {
            hide_window(Some(hwnd));
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
                    if is_current && !copy_data.lpData.is_null() && copy_data.cbData > 0 {
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
