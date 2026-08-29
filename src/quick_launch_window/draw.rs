//! 検索窓・候補リストの描画本体。

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, HDC, HFONT,
    PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED};

use super::badge::{action_color, azure_icon_color, azure_icon_kind, badge_color};
use super::draw_icons::{
    FaviconFallback, backdrop_tint, draw_azure_icon, draw_command_icon, draw_favicon_icon,
    draw_icon_backdrop, draw_path_icon, draw_window_icon,
};
use super::layout::{scale, weekday_label};
use super::{
    ACCENT, BACKGROUND, BADGE_WIDTH, EDIT_HEIGHT, ICON_LEFT, PADDING, STATE, SURFACE,
    SURFACE_HOVER, TEXT_LEFT, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, rgb,
};
use crate::quick_launch::{Action, Entry};

pub(super) fn paint_window(window: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(window, &mut paint);
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut client);
        let (dpi, background, surface, badge, detail_font, everything_flags, everything_active) =
            STATE.with(|state| {
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
            let surface_pen = CreatePen(PS_SOLID, 1, SURFACE);
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
                draw_badge(hdc, badge, search, dpi, detail_font);
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
            draw_text_centered(hdc, badge, &mut text_rect);
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

/// 絞り込みなし一覧の区分見出し行を描く。アイコンは持たず、小さめの
/// ラベルを行の下寄せで置き、下端に 1px の区切り線を引いて次の項目行と
/// 分ける (Spotlight のセクション見出しに近い見た目)。
pub(super) unsafe fn draw_section_header(
    hdc: HDC,
    label: &str,
    rect: RECT,
    dpi: u32,
    detail_font: Option<HFONT>,
) {
    unsafe {
        if let Some(font) = detail_font {
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, TEXT_MUTED);
            let mut text_rect = RECT {
                left: rect.left + scale(ICON_LEFT, dpi),
                top: rect.top,
                right: rect.right - scale(8, dpi),
                bottom: rect.bottom - scale(6, dpi),
            };
            draw_text(hdc, label, &mut text_rect);
            SelectObject(hdc, old_font);
        }

        let divider = CreateSolidBrush(SURFACE_HOVER);
        let divider_rect = RECT {
            left: rect.left + scale(ICON_LEFT, dpi),
            top: rect.bottom - scale(1, dpi),
            right: rect.right - scale(8, dpi),
            bottom: rect.bottom,
        };
        FillRect(hdc, &divider_rect, divider);
        let _ = DeleteObject(divider.into());
    }
}

pub(super) unsafe fn draw_list_item(draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let Some((row, entry, empty_message, name_font, detail_font, dpi, badge)) =
        STATE.with(|state| {
            let state = state.borrow();
            let row = state.rows.get(draw.itemID as usize).copied()?;
            let entry = match row {
                super::RowKind::Item(index) => Some(state.results.get(index)?.clone()),
                super::RowKind::Header(_) | super::RowKind::Message => None,
            };
            Some((
                row,
                entry,
                state.empty_message.clone(),
                state.name_font,
                state.detail_font,
                state.dpi,
                state.badge,
            ))
        })
    else {
        return;
    };

    unsafe {
        let selected = draw.itemState.0 & ODS_SELECTED.0 != 0;
        let background = CreateSolidBrush(BACKGROUND);
        FillRect(draw.hDC, &draw.rcItem, background);
        let _ = DeleteObject(background.into());

        if let super::RowKind::Header(label) = row {
            draw_section_header(draw.hDC, label, draw.rcItem, dpi, detail_font);
            return;
        }

        let Some(entry) = entry else {
            if let (Some(message), Some(font)) = (empty_message, detail_font) {
                let old = SelectObject(draw.hDC, font.into());
                SetBkMode(draw.hDC, TRANSPARENT);
                SetTextColor(draw.hDC, TEXT_SECONDARY);
                let mut rect = draw.rcItem;
                rect.left += scale(16, dpi);
                draw_text(draw.hDC, &message, &mut rect);
                SelectObject(draw.hDC, old);
            }
            return;
        };

        // 選択行はカード風に少し内側へ収め、角を丸めて他の行から浮かせる
        if selected {
            let accent_color = badge.map_or(ACCENT, badge_color);
            let inset = scale(2, dpi);
            let card = RECT {
                left: draw.rcItem.left + inset,
                top: draw.rcItem.top + scale(1, dpi),
                right: draw.rcItem.right - inset,
                bottom: draw.rcItem.bottom - scale(1, dpi),
            };
            // 枠線を地の SURFACE_HOVER より一段明るくし、選択カードに
            // 「押せる」輪郭を持たせる (計画: 選択行の演出強化)。
            let card_pen_color = rgb(58, 90, 110);
            let card_brush = CreateSolidBrush(SURFACE_HOVER);
            let card_pen = CreatePen(PS_SOLID, 1, card_pen_color);
            let old_brush = SelectObject(draw.hDC, card_brush.into());
            let old_pen = SelectObject(draw.hDC, card_pen.into());
            let radius = scale(8, dpi);
            let _ = RoundRect(
                draw.hDC,
                card.left,
                card.top,
                card.right,
                card.bottom,
                radius,
                radius,
            );
            SelectObject(draw.hDC, old_brush);
            SelectObject(draw.hDC, old_pen);
            let _ = DeleteObject(card_brush.into());
            let _ = DeleteObject(card_pen.into());

            let accent = CreateSolidBrush(accent_color);
            let accent_rect = RECT {
                left: card.left,
                top: card.top + scale(6, dpi),
                right: card.left + scale(3, dpi),
                bottom: card.bottom - scale(6, dpi),
            };
            FillRect(draw.hDC, &accent_rect, accent);
            let _ = DeleteObject(accent.into());
        }

        if let Some(kind) = azure_icon_kind(badge, &entry.path) {
            let color = azure_icon_color(kind);
            draw_icon_backdrop(draw.hDC, color, draw.rcItem, dpi);
            draw_azure_icon(draw.hDC, kind, color, draw.rcItem, dpi, name_font);
        } else {
            draw_icon_backdrop(draw.hDC, action_color(&entry.action), draw.rcItem, dpi);
            match entry.action {
                Action::OpenFolder(_) | Action::OpenWithDefaultHandler | Action::LaunchApp => {
                    draw_path_icon(draw.hDC, &entry.path, draw.rcItem, dpi)
                }
                Action::FocusWindow(hwnd) => {
                    draw_window_icon(draw.hDC, HWND(hwnd as *mut _), draw.rcItem, dpi)
                }
                Action::OpenUrl(_) => draw_favicon_icon(
                    draw.hDC,
                    &entry.path,
                    draw.rcItem,
                    dpi,
                    FaviconFallback::Bookmark,
                ),
                Action::FocusBrowserTab(_) => draw_favicon_icon(
                    draw.hDC,
                    &entry.path,
                    draw.rcItem,
                    dpi,
                    FaviconFallback::Tab,
                ),
                Action::ReplaceQuery(_)
                | Action::AzureLiveWorkItemSearch(_)
                | Action::AzureLivePullRequestSearch { .. }
                | Action::AzureLivePipelineSearch { .. }
                | Action::AzureSuggestPriorities => {
                    draw_command_icon(draw.hDC, draw.rcItem, dpi, name_font)
                }
            }
        }
        SetBkMode(draw.hDC, TRANSPARENT);
        let text_left = draw.rcItem.left + scale(TEXT_LEFT, dpi);
        let text_right = draw.rcItem.right - scale(8, dpi);

        if let Some(font) = name_font {
            let old = SelectObject(draw.hDC, font.into());
            SetTextColor(draw.hDC, TEXT_PRIMARY);
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(2, dpi),
                right: text_right,
                bottom: draw.rcItem.top + scale(23, dpi),
            };
            let label = crate::git::with_branch(&entry.name, entry.branch.as_deref());
            draw_text(draw.hDC, &label, &mut rect);
            SelectObject(draw.hDC, old);
        }

        if let Some(font) = detail_font {
            let old = SelectObject(draw.hDC, font.into());
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(20, dpi),
                right: text_right,
                bottom: draw.rcItem.bottom - scale(1, dpi),
            };
            // breadcrumb を主、path を一段暗い色の補足として続ける。
            // 両方を同じ色で "—" 連結していた旧描画は、長いパスがあると
            // breadcrumb まで埋もれて読みにくかった (計画時の指摘)。
            SetTextColor(draw.hDC, TEXT_SECONDARY);
            let primary = detail_primary(&entry);
            let primary_width = measured_width(draw.hDC, &primary, &rect);
            draw_text(draw.hDC, &primary, &mut rect);
            if let Some(path) = detail_secondary(&entry) {
                let mut secondary_rect = rect;
                secondary_rect.left = (rect.left + primary_width).min(rect.right);
                if secondary_rect.left < secondary_rect.right {
                    SetTextColor(draw.hDC, TEXT_MUTED);
                    draw_text(draw.hDC, &path, &mut secondary_rect);
                }
            }
            SelectObject(draw.hDC, old);
        }
    }
}

/// 詳細行の主要テキスト。breadcrumb があればそれを、無ければパスを出す
/// (ウィンドウ項目のように breadcrumb しか持たない候補もあるため)。
pub(super) fn detail_primary(entry: &Entry) -> String {
    if entry.breadcrumb.is_empty() {
        entry.path.clone()
    } else {
        entry.breadcrumb.clone()
    }
}

/// breadcrumb の右側に淡色で続けるパス。breadcrumb が無い、または
/// パスと同じ内容を primary で既に出している場合は None。
/// Azure DevOps 項目は breadcrumb 自体に作成者などの有益情報を持つため、
/// 冗長でしかない URL は表示しない。
pub(super) fn detail_secondary(entry: &Entry) -> Option<String> {
    (!entry.breadcrumb.is_empty()
        && !entry.path.is_empty()
        && !entry.breadcrumb.starts_with("Azure DevOps —"))
    .then(|| format!("   ›   {}", entry.path))
}

/// リストボックスへ渡すプレーンラベル用の補足テキスト。
/// 実描画は `draw_list_item` (オーナードロー) が行うため、ここは
/// アクセシビリティ・内部データとしての文字列表現に過ぎない。
pub(super) fn entry_context(entry: &Entry) -> String {
    match detail_secondary(entry) {
        Some(secondary) => format!("{}{}", detail_primary(entry), secondary),
        None => detail_primary(entry),
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
