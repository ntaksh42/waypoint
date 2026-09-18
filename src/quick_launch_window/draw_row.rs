//! 候補リストの行描画。
//!
//! `draw.rs` が 500 行規約を超えたため分離した。検索窓まわりの描画
//! (`draw.rs`) に対して、こちらは 1 行ぶんの描画と、その行に出す
//! 文字列の組み立てを受け持つ。

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, HDC, HFONT, PS_SOLID, RoundRect,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED};

use super::badge::{action_color, azure_icon_color, azure_icon_kind, badge_color};
use super::draw::{draw_text, measured_width};
use super::draw_icons::{
    FaviconFallback, draw_azure_icon, draw_claude_code_icon, draw_command_icon, draw_favicon_icon,
    draw_icon_backdrop, draw_path_icon, draw_window_icon,
};
use super::layout::scale;
use super::{
    ACCENT, BACKGROUND, ICON_LEFT, SELECTED_BG, SELECTED_BORDER, STATE, SURFACE_HOVER, TEXT_LEFT,
    TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::quick_launch::{Action, Entry};

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
    let Some((row, entry, empty_message, name_font, detail_font, dpi, badge, highlight_term)) =
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
                state.highlight_term.clone(),
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

        // 項目同士の境界が背景色だけでは判別しづらいため、非選択行の下端に
        // ごく薄い区切り線を引く。選択行はカードが行全体を覆うので不要。
        if !selected {
            let divider = CreateSolidBrush(SURFACE_HOVER);
            let divider_rect = RECT {
                left: draw.rcItem.left + scale(ICON_LEFT, dpi),
                top: draw.rcItem.bottom - scale(1, dpi),
                right: draw.rcItem.right - scale(8, dpi),
                bottom: draw.rcItem.bottom,
            };
            FillRect(draw.hDC, &divider_rect, divider);
            let _ = DeleteObject(divider.into());
        }

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
            // 背景 (BACKGROUND) とのコントラスト比 2.7:1 まで引き上げた
            // SELECTED_BG で塗り、枠線もそれよりさらに明るい SELECTED_BORDER
            // にして、リスト内移動時に選択位置が即座に分かるようにする。
            let card_brush = CreateSolidBrush(SELECTED_BG);
            let card_pen = CreatePen(PS_SOLID, 1, SELECTED_BORDER);
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

            // アクセントバーは種別色 (badge 由来) のまま、視認性向上のため
            // 3px→4px へ太くする。
            let accent = CreateSolidBrush(accent_color);
            let accent_rect = RECT {
                left: card.left,
                top: card.top + scale(6, dpi),
                right: card.left + scale(4, dpi),
                bottom: card.bottom - scale(6, dpi),
            };
            FillRect(draw.hDC, &accent_rect, accent);
            let _ = DeleteObject(accent.into());
        }

        // `cc ` のフォルダ候補 (`claude_code_folder_entries`) は検索欄を補完する
        // `ReplaceQuery` だが、実体はフォルダを指す。他のフォルダ候補や `az` の
        // サブコマンド補完 (path を持たない `ReplaceQuery`) と見分けられるよう、
        // "CC" グリフを Claude Code のバッジ色 (緑寄り) で描く。
        let is_claude_code_folder_candidate =
            matches!(entry.action, Action::ReplaceQuery(_)) && !entry.path.is_empty();
        if let Some(kind) = azure_icon_kind(badge, &entry.path) {
            let color = azure_icon_color(kind);
            draw_icon_backdrop(draw.hDC, color, draw.rcItem, dpi);
            draw_azure_icon(draw.hDC, kind, color, draw.rcItem, dpi, name_font);
        } else if is_claude_code_folder_candidate {
            let color = badge_color("CLAUDE CODE");
            draw_icon_backdrop(draw.hDC, color, draw.rcItem, dpi);
            draw_claude_code_icon(draw.hDC, color, draw.rcItem, dpi, name_font);
        } else {
            draw_icon_backdrop(draw.hDC, action_color(&entry.action), draw.rcItem, dpi);
            match entry.action {
                Action::OpenFolder(_)
                | Action::OpenWithDefaultHandler
                | Action::LaunchApp
                | Action::OpenInTerminal
                | Action::OpenInEditor(_)
                | Action::OpenClaudeCode(_) => {
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
                Action::WebSearch => draw_favicon_icon(
                    draw.hDC,
                    &entry.path,
                    draw.rcItem,
                    dpi,
                    FaviconFallback::Web,
                ),
                Action::ReplaceQuery(_)
                | Action::AzureLiveWorkItemSearch(_)
                | Action::AzureLivePullRequestSearch { .. }
                | Action::AzureLivePipelineSearch { .. }
                | Action::AzureOptimize
                | Action::OpenSettings
                | Action::OpenHelp
                | Action::KillProcess(_) => {
                    draw_command_icon(draw.hDC, draw.rcItem, dpi, name_font)
                }
            }
        }
        SetBkMode(draw.hDC, TRANSPARENT);
        let text_left = draw.rcItem.left + scale(TEXT_LEFT, dpi);
        let text_right = draw.rcItem.right - scale(8, dpi);

        if let Some(font) = name_font {
            let old = SelectObject(draw.hDC, font.into());
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(2, dpi),
                right: text_right,
                bottom: draw.rcItem.top + scale(23, dpi),
            };
            let label = crate::git::with_branch(&entry.name, entry.branch.as_deref());
            let ranges = crate::quick_launch::highlight_ranges(&entry.name, &highlight_term);
            super::highlight::draw_text_highlighted(
                draw.hDC,
                &label,
                &ranges,
                &mut rect,
                TEXT_PRIMARY,
                ACCENT,
            );
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
