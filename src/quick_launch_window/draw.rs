//! 検索窓・候補リストの描画本体。

use super::badge::{badge_color, shows_live_search_hint};
use super::draw_icons::backdrop_tint;
use super::layout::{scale, weekday_label};
use super::{ACCENT, BADGE_WIDTH, EDIT_HEIGHT, PADDING, STATE, SURFACE_HOVER, TEXT_SECONDARY};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, HDC, HFONT,
    PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

pub(super) fn paint_window(window: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(window, &mut paint);
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut client);
        let (
            dpi,
            background,
            surface,
            badge,
            detail_font,
            everything_flags,
            everything_active,
            live_hint,
        ) = STATE.with(|state| {
            let state = state.borrow();
            let badge = if state.copy_feedback {
                Some("COPIED")
            } else {
                state.badge
            };
            (
                state.dpi,
                state.background_brush,
                state.surface_brush,
                badge,
                state.detail_font,
                state.everything_flags,
                state.everything_active,
                state.live_search_hint,
            )
        });
        if let Some(background) = background {
            FillRect(hdc, &client, background);
        }
        if let Some(surface) = surface {
            let padding = scale(PADDING, dpi);
            let edit_height = scale(EDIT_HEIGHT, dpi);
            let search = RECT {
                left: padding,
                top: padding,
                right: client.right - padding,
                bottom: padding + edit_height,
            };
            // 枠線を塗りつぶしと同じ SURFACE にすると検索窓の輪郭が消えて
            // 背景と一体化して見えるため、地よりわずかに明るい色で縁取る。
            let surface_pen = CreatePen(PS_SOLID, 1, SURFACE_HOVER);
            let old_pen = SelectObject(hdc, surface_pen.into());
            let old_brush = SelectObject(hdc, surface.into());
            let radius = scale(10, dpi);
            let _ = RoundRect(
                hdc,
                search.left,
                search.top,
                search.right,
                search.bottom,
                radius,
                radius,
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(surface_pen.into());

            if let Some(badge) = badge {
                draw_badge(hdc, badge, search, dpi, detail_font, live_hint);
                if everything_active {
                    draw_everything_flag_badges(hdc, everything_flags, search, dpi, detail_font);
                }
            } else {
                draw_clock(hdc, search, dpi, detail_font);
            }
        }
        let _ = EndPaint(window, &paint);
    }
}

/// 検索窓の右端に、アクティブなプレフィックスモードの名前を丸バッジで描く。
pub(super) unsafe fn draw_badge(
    hdc: HDC,
    badge: &str,
    search: RECT,
    dpi: u32,
    detail_font: Option<HFONT>,
    // 現在の入力で `Ctrl+Enter` の Live 検索が実際に成立するか。
    // `az project` / `az optimize` は検索対象を持たず Live にならないので、
    // バッジも通常表示に戻す (表示と挙動の食い違いを残さない)
    live_hint: bool,
) {
    unsafe {
        let color = badge_color(badge);
        let height = scale(20, dpi);
        let width = scale(BADGE_WIDTH, dpi) - scale(16, dpi);
        let rect = RECT {
            left: search.right - scale(10, dpi) - width,
            top: search.top + (search.bottom - search.top - height) / 2,
            right: search.right - scale(10, dpi),
            bottom: search.top + (search.bottom - search.top - height) / 2 + height,
        };
        // バッジ地はカードと同じ低彩度トーンにし、縁取りだけモード色を残す。
        // 塗りつぶし全体を原色にすると検索窓から浮いて見えるため
        // (計画: 検索窓・バッジ周りの調整)。
        let brush = CreateSolidBrush(backdrop_tint(color));
        let radius = height / 2;
        let old_brush = SelectObject(hdc, brush.into());
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_pen = SelectObject(hdc, pen.into());
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());

        if let Some(font) = detail_font {
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, color);
            let mut text_rect = rect;
            let label = if live_hint && shows_live_search_hint(Some(badge)) {
                "Ctrl+Enter  Live"
            } else {
                badge
            };
            draw_text_centered(hdc, label, &mut text_rect);
            SelectObject(hdc, old_font);
        }
    }
}

/// バッジ非表示中の検索窓の右端に、日付・曜日・現在時刻を淡色で描く。
pub(super) unsafe fn draw_clock(hdc: HDC, search: RECT, dpi: u32, detail_font: Option<HFONT>) {
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let text = format!(
        "{}/{} {} {:02}:{:02}",
        time.wMonth,
        time.wDay,
        weekday_label(time.wDayOfWeek),
        time.wHour,
        time.wMinute
    );
    unsafe {
        let Some(font) = detail_font else { return };
        let width = scale(BADGE_WIDTH, dpi) - scale(16, dpi);
        let mut rect = RECT {
            left: search.right - scale(10, dpi) - width,
            top: search.top,
            right: search.right - scale(10, dpi),
            bottom: search.bottom,
        };
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, TEXT_SECONDARY);
        draw_text_centered(hdc, &text, &mut rect);
        SelectObject(hdc, old_font);
    }
}

/// `f ` モードでアクティブな Everything 検索フラグを、モードバッジの
/// 左側へ小さいピルとして並べて描く (Alt+C/W/R でトグルした状態の可視化)。
pub(super) unsafe fn draw_everything_flag_badges(
    hdc: HDC,
    flags: u32,
    search: RECT,
    dpi: u32,
    detail_font: Option<HFONT>,
) {
    let active: Vec<&str> = [
        (crate::everything::MATCH_CASE, "Cc"),
        (crate::everything::MATCH_WHOLE_WORD, "W"),
        (crate::everything::REGEX, ".*"),
    ]
    .into_iter()
    .filter(|(flag, _)| flags & flag != 0)
    .map(|(_, label)| label)
    .collect();
    if active.is_empty() {
        return;
    }

    unsafe {
        let height = scale(20, dpi);
        let gap = scale(6, dpi);
        let badge_left_edge =
            search.right - scale(10, dpi) - (scale(BADGE_WIDTH, dpi) - scale(16, dpi));
        let mut right = badge_left_edge - gap;
        for label in active {
            let width = scale(10, dpi) * 2 + scale(8, dpi) * label.chars().count() as i32;
            let rect = RECT {
                left: right - width,
                top: search.top + (search.bottom - search.top - height) / 2,
                right,
                bottom: search.top + (search.bottom - search.top - height) / 2 + height,
            };
            let brush = CreateSolidBrush(SURFACE_HOVER);
            let pen = CreatePen(PS_SOLID, 1, ACCENT);
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = height / 2;
            let _ = RoundRect(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                radius,
                radius,
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());

            if let Some(font) = detail_font {
                let old_font = SelectObject(hdc, font.into());
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, ACCENT);
                let mut text_rect = rect;
                draw_text_centered(hdc, label, &mut text_rect);
                SelectObject(hdc, old_font);
            }

            right = rect.left - gap;
        }
    }
}

/// `text` を `bounds` の幅制約 (`DT_END_ELLIPSIS` 込み) で描いたときの実幅を測る。
/// 実際には描画しない (`DT_CALCRECT`)。詳細行で breadcrumb (primary) の直後に
/// path (secondary) を続けて描く開始 x 座標を決めるために使う。
pub(super) unsafe fn measured_width(hdc: HDC, text: &str, bounds: &RECT) -> i32 {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut calc_rect = *bounds;
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            &mut calc_rect,
            DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_CALCRECT,
        );
    }
    calc_rect.right - calc_rect.left
}

pub(super) unsafe fn draw_text(hdc: HDC, text: &str, rect: &mut RECT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

/// バッジの文字を水平・垂直とも中央揃えで描く。
pub(super) unsafe fn draw_text_centered(hdc: HDC, text: &str, rect: &mut RECT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
        );
    }
}
