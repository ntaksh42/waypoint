//! ウィンドウ配置・DPI・フォント。

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DeleteObject, FW_NORMAL, FW_SEMIBOLD, GetMonitorInfoW, HFONT, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::Controls::EM_SETMARGINS;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, MoveWindow, WM_SETFONT};
use windows::core::w;

use super::{EDIT_HEIGHT, HEADER_HEIGHT, PADDING, ROW_HEIGHT, RowKind, STATE, WINDOW_WIDTH};

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

pub(super) fn position_window(window: HWND, monitor_window: HWND, rows_height: i32, dpi: u32) {
    unsafe {
        let monitor = MonitorFromWindow(monitor_window, MONITOR_DEFAULTTONEAREST);
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
        // `height - (PADDING + EDIT_HEIGHT + 6) - PADDING` に決まる。
        // ここで組み立てる height は逆算で、その式が rows_height と一致する
        // よう PADDING を 2 回 (検索窓の上下) 差し引いた分だけ足す。
        let height = scale(PADDING * 2 + EDIT_HEIGHT + 6 + rows_height, dpi);
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = MoveWindow(window, x, y, width, height, true);
    }
}

pub(super) fn apply_dpi(window: HWND, dpi: u32) {
    let dpi = dpi.max(96);
    let (edit, old_fonts, fonts) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.dpi == dpi && state.edit_font.is_some() {
            return (
                state.edit,
                Vec::new(),
                (state.edit_font, state.name_font, state.detail_font),
            );
        }
        let old_fonts = [state.edit_font, state.name_font, state.detail_font]
            .into_iter()
            .flatten()
            .collect();
        state.dpi = dpi;
        state.edit_font = create_font(scale(14, dpi), FW_NORMAL.0 as i32);
        state.name_font = create_font(scale(14, dpi), FW_SEMIBOLD.0 as i32);
        state.detail_font = create_font(scale(12, dpi), FW_NORMAL.0 as i32);
        (
            state.edit,
            old_fonts,
            (state.edit_font, state.name_font, state.detail_font),
        )
    });

    unsafe {
        if let (Some(edit), Some(font)) = (edit, fonts.0) {
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

pub(super) fn create_font(pixel_height: i32, weight: i32) -> Option<HFONT> {
    let font = unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            weight,
            0,
            0,
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
