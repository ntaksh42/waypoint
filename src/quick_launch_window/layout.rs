//! ウィンドウ配置・DPI・フォント。

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DeleteObject, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, GetMonitorInfoW, HFONT, InvalidateRect,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, EM_SETMARGINS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, MoveWindow, WM_SETFONT,
};
use windows::core::w;

use crate::config::MonitorChoice;

use super::{
    EDIT_HEIGHT, FOOTER_GAP, FOOTER_HEIGHT, HEADER_HEIGHT, PADDING, ROW_HEIGHT, RowKind, STATE,
    WINDOW_WIDTH,
};

/// `rows` (見出し・項目・メッセージ行の並び) を実際に描画したときの合計高さを
/// 見積もる (DPI 適用前、論理ピクセル)。見出し行は `HEADER_HEIGHT`、それ以外
/// (項目・メッセージ) は `ROW_HEIGHT` で `WM_MEASUREITEM` (dispatch.rs) と
/// 同じ内訳にする。`max_rows` を超える分は表示されず (`MoveWindow` でリスト
/// ボックスの高さが頭打ちになる) スクロール対象になるので、そこで打ち切る。
/// 非同期検索の応答待ちなど `rows` が一時的に空になる瞬間もあるため、
/// 最低でも 1 行分は確保し、結果が届くたびにウィンドウが最小サイズへ
/// 縮んでからまた伸びるチラつきを避ける。
pub(super) fn rows_height(rows: &[RowKind], max_rows: usize) -> i32 {
    let height = rows.iter().take(max_rows.max(1)).fold(0, |acc, row| {
        acc + match row {
            RowKind::Header(_) => HEADER_HEIGHT,
            RowKind::Item(_) | RowKind::Message => ROW_HEIGHT,
        }
    });
    height.max(ROW_HEIGHT)
}

/// 表示先のモニターを返す (FR-9.12.1)。設定が `Cursor` ならマウスカーソル
/// のあるモニター、`Primary` ならプライマリモニター。カーソル位置が取れない
/// 場合はプライマリへ落とす。
///
/// DPI 取得 (`target_monitor_dpi`) と配置 (`position_window`) が別々に
/// モニターを選ぶと、寸法と表示先がずれうるので経路を 1 本にまとめてある。
fn target_monitor() -> windows::Win32::Graphics::Gdi::HMONITOR {
    let choice = STATE.with(|state| state.borrow().monitor);
    let point = match choice {
        MonitorChoice::Primary => POINT { x: 0, y: 0 },
        MonitorChoice::Cursor => {
            let mut cursor = POINT::default();
            if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
                cursor
            } else {
                POINT { x: 0, y: 0 }
            }
        }
    };
    // MONITOR_DEFAULTTOPRIMARY: カーソルがどのモニターにも属さない座標に
    // ある場合 (モニター切断直後など) もプライマリへ落ちる
    unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY) }
}

pub(super) fn target_monitor_dpi() -> u32 {
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    let result = unsafe {
        windows::Win32::UI::HiDpi::GetDpiForMonitor(
            target_monitor(),
            windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI,
            &mut dpi_x,
            &mut dpi_y,
        )
    };
    if result.is_ok() { dpi_x.max(96) } else { 96 }
}

pub(super) fn position_window(window: HWND, rows_height: i32, dpi: u32) {
    unsafe {
        let monitor = target_monitor();
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut work = RECT::default();
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            work = info.rcWork;
        } else {
            let _ = GetClientRect(window, &mut work);
        }
        let width = scale(WINDOW_WIDTH, dpi);
        // リストボックスの実高さは WM_SIZE (dispatch.rs) で
        // `height - (PADDING + EDIT_HEIGHT + 6) - (FOOTER_GAP + FOOTER_HEIGHT)` に決まる。
        // ここで組み立てる height は逆算で、その式が rows_height と一致する
        // よう、検索窓の帯と下端の操作ヒント帯の分だけ足す。
        let height = scale(
            PADDING + EDIT_HEIGHT + 6 + rows_height + FOOTER_GAP + FOOTER_HEIGHT,
            dpi,
        );
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = MoveWindow(window, x, y, width, height, true);
    }
}

pub(super) fn apply_dpi(window: HWND, dpi: u32) {
    let dpi = dpi.max(96);
    let (edit, old_fonts, edit_font) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.dpi == dpi && state.edit_font.is_some() {
            return (state.edit, Vec::new(), state.edit_font);
        }
        let old_fonts = [
            state.edit_font,
            state.name_font,
            state.detail_font,
            state.highlight_font,
            state.tag_font,
            state.header_font,
        ]
        .into_iter()
        .flatten()
        .collect();
        state.dpi = dpi;
        state.edit_font = create_font(scale(16, dpi), FW_NORMAL.0 as i32, false);
        state.name_font = create_font(scale(15, dpi), FW_SEMIBOLD.0 as i32, false);
        state.detail_font = create_font(scale(13, dpi), FW_NORMAL.0 as i32, false);
        // 一致箇所は色だけでは選択行で区別しづらいため、下線付き太字にする
        state.highlight_font = create_font(scale(15, dpi), FW_BOLD.0 as i32, true);
        state.tag_font = create_font(scale(12, dpi), FW_NORMAL.0 as i32, false);
        state.header_font = create_font(scale(11, dpi), FW_BOLD.0 as i32, false);
        (state.edit, old_fonts, state.edit_font)
    });

    unsafe {
        if let (Some(edit), Some(font)) = (edit, edit_font) {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let left = scale(2, dpi) as u32;
            let right = scale(8, dpi) as u32;
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                EM_SETMARGINS,
                Some(WPARAM(3)),
                Some(LPARAM(((right << 16) | left) as isize)),
            );
        }
        for font in old_fonts {
            let _ = DeleteObject(font.into());
        }
        let _ = InvalidateRect(Some(window), None, true);
    }
}

/// 空の検索欄に淡色で出す案内文 (cue banner) を設定する。
/// 何を検索できるかを、入力前に一目で分かるようにする。
pub(super) fn set_search_placeholder(edit: HWND) {
    let text = w!("Search folders, windows, apps\u{2026}");
    unsafe {
        // wParam = 1: フォーカス中も出す。検索画面は開いた瞬間から入力欄に
        // フォーカスがあるため、0 だと案内文が一度も見えない
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_SETCUEBANNER,
            Some(WPARAM(1)),
            Some(LPARAM(text.as_ptr() as isize)),
        );
    }
}

pub(super) fn create_font(pixel_height: i32, weight: i32, underline: bool) -> Option<HFONT> {
    let font = unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            weight,
            0,
            u32::from(underline),
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0.into(),
            w!("Segoe UI Variable Text"),
        )
    };
    (!font.is_invalid()).then_some(font)
}

pub(super) fn apply_window_chrome(window: HWND) {
    unsafe {
        let dark = 1i32;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&corner).cast(),
            size_of_val(&corner) as u32,
        );
    }
}

pub(super) fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi.max(96) as i32 / 96
}

/// 曜日番号 (`SYSTEMTIME::wDayOfWeek`、0=日曜) を英語 3 文字表記へ変換する。
pub(super) fn weekday_label(day_of_week: u16) -> &'static str {
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    NAMES[day_of_week as usize % 7]
}
