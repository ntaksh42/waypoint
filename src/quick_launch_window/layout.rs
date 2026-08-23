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

use super::{EDIT_HEIGHT, PADDING, ROW_HEIGHT, STATE, WINDOW_WIDTH};

pub(super) fn position_window(window: HWND, monitor_window: HWND, rows: usize, dpi: u32) {
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
        let height = scale(
            PADDING * 3 + EDIT_HEIGHT + 6 + ROW_HEIGHT * rows as i32 + 36,
            dpi,
        );
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
        state.detail_font = create_font(scale(11, dpi), FW_NORMAL.0 as i32);
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
