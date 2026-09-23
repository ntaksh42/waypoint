//! 候補リストの行描画。
//!
//! `draw.rs` が 500 行規約を超えたため分離した。検索窓まわりの描画
//! (`draw.rs`) に対して、こちらは 1 行ぶんの描画と、その行に出す
//! 文字列の組み立てを受け持つ。見出し行とタグの部品は `row_parts.rs`。

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, HFONT, PS_SOLID, RoundRect, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED};

use super::badge::{action_tag, action_tag_color, action_verb, azure_icon_color, azure_icon_kind};
use super::draw::{draw_text, draw_text_path, measured_width};
use super::draw_icons::{
    FaviconFallback, draw_azure_icon, draw_command_icon, draw_favicon_icon, draw_path_icon,
    draw_window_icon,
};
use super::layout::scale;
use super::row_parts::{TagStyle, draw_section_header, draw_tag_right_aligned};
use super::{
    ACCENT, BACKGROUND, BRANCH_BORDER, BRANCH_TEXT, SELECTED_BG, SELECTED_HIGHLIGHT,
    SELECTED_TAG_BORDER, SELECTED_TEXT_MUTED, SELECTED_TEXT_SECONDARY, STATE, TAG_BG, TAG_RIGHT,
    TEXT_LEFT, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::quick_launch::{Action, Entry};

/// 1 行の描画に使うフォント一式。STATE の借用を描画前に解くため値で持ち出す。
#[derive(Clone, Copy)]
struct RowFonts {
    name: Option<HFONT>,
    detail: Option<HFONT>,
    highlight: Option<HFONT>,
    tag: Option<HFONT>,
    header: Option<HFONT>,
}

/// `rows[header_row]` の見出しに続く項目行の数 (次の見出しまで)。
pub(super) fn section_count(rows: &[super::RowKind], header_row: usize) -> usize {
    rows.iter()
        .skip(header_row + 1)
        .take_while(|row| matches!(row, super::RowKind::Item(_)))
        .count()
}

pub(super) unsafe fn draw_list_item(draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let Some((row, entry, empty_message, fonts, dpi, badge, highlight_term, count)) =
        STATE.with(|state| {
            let state = state.borrow();
            let row = state.rows.get(draw.itemID as usize).copied()?;
            let entry = match row {
                super::RowKind::Item(index) => Some(state.results.get(index)?.clone()),
                super::RowKind::Header(_) | super::RowKind::Message => None,
            };
            let count = match row {
                super::RowKind::Header(_) => section_count(&state.rows, draw.itemID as usize),
                _ => 0,
            };
            Some((
                row,
                entry,
                state.empty_message.clone(),
                RowFonts {
                    name: state.name_font,
                    detail: state.detail_font,
                    highlight: state.highlight_font,
                    tag: state.tag_font,
                    header: state.header_font,
                },
                state.dpi,
                state.badge,
                state.highlight_term.clone(),
                count,
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
            draw_section_header(
                draw.hDC,
                label,
                count,
                draw.rcItem,
                dpi,
                fonts.header,
                fonts.tag,
            );
            return;
        }

        let Some(entry) = entry else {
            if let (Some(message), Some(font)) = (empty_message, fonts.detail) {
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

        // 選択行は行全体を青で塗る。旧描画 (暗い灰色 + 枠線 + 左端のバー) は
        // 地との差が小さく、キーボードで移動したときに選択位置を見失いやすかった。
        // 非選択行の区切り線も、見出しと紛れて雑然とするため引かない。
        if selected {
            let card = RECT {
                left: draw.rcItem.left,
                top: draw.rcItem.top + scale(1, dpi),
                right: draw.rcItem.right,
                bottom: draw.rcItem.bottom - scale(1, dpi),
            };
            let card_brush = CreateSolidBrush(SELECTED_BG);
            let card_pen = CreatePen(PS_SOLID, 1, SELECTED_BG);
            let old_brush = SelectObject(draw.hDC, card_brush.into());
            let old_pen = SelectObject(draw.hDC, card_pen.into());
            let radius = scale(12, dpi);
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
        }

        draw_entry_icon(draw, &entry, badge, dpi, fonts.name);
        let text_right = draw_trailing_tags(draw, &entry, badge, selected, dpi, fonts.tag);

        SetBkMode(draw.hDC, TRANSPARENT);
        let text_left = draw.rcItem.left + scale(TEXT_LEFT, dpi);

        if let Some(font) = fonts.name {
            let old = SelectObject(draw.hDC, font.into());
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(5, dpi),
                right: text_right,
                bottom: draw.rcItem.top + scale(27, dpi),
            };
            let ranges = crate::quick_launch::highlight_ranges(&entry.name, &highlight_term);
            let highlight_color = if selected { SELECTED_HIGHLIGHT } else { ACCENT };
            super::highlight::draw_text_highlighted(
                draw.hDC,
                &entry.name,
                &ranges,
                &mut rect,
                TEXT_PRIMARY,
                highlight_color,
                fonts.highlight,
            );
            SelectObject(draw.hDC, old);
        }

        if let Some(font) = fonts.detail {
            let old = SelectObject(draw.hDC, font.into());
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(26, dpi),
                right: text_right,
                bottom: draw.rcItem.top + scale(44, dpi),
            };
            // breadcrumb を主、path を一段暗い色の補足として続ける。
            // 長いパスは中央を省略し、末尾のフォルダ名・ファイル名を残す。
            let (primary_color, secondary_color) = if selected {
                (SELECTED_TEXT_SECONDARY, SELECTED_TEXT_MUTED)
            } else {
                (TEXT_SECONDARY, TEXT_MUTED)
            };
            SetTextColor(draw.hDC, primary_color);
            let primary = detail_primary(&entry);
            let primary_width = measured_width(draw.hDC, &primary, &rect);
            if entry.breadcrumb.is_empty() {
                draw_text_path(draw.hDC, &primary, &mut rect);
            } else {
                draw_text(draw.hDC, &primary, &mut rect);
            }
            if let Some(path) = detail_secondary(&entry) {
                let mut secondary_rect = rect;
                secondary_rect.left = (rect.left + primary_width).min(rect.right);
                if secondary_rect.left < secondary_rect.right {
                    SetTextColor(draw.hDC, secondary_color);
                    draw_text_path(draw.hDC, &path, &mut secondary_rect);
                }
            }
            SelectObject(draw.hDC, old);
        }
    }
}

/// 行頭のアイコンを描く。
unsafe fn draw_entry_icon(
    draw: &DRAWITEMSTRUCT,
    entry: &Entry,
    badge: Option<&str>,
    dpi: u32,
    name_font: Option<HFONT>,
) {
    unsafe {
        if let Some(kind) = azure_icon_kind(badge, &entry.path) {
            let color = azure_icon_color(kind);
            draw_azure_icon(draw.hDC, kind, color, draw.rcItem, dpi, name_font);
            return;
        }
        match entry.action {
            Action::OpenFolder(_)
            | Action::OpenWithDefaultHandler
            | Action::LaunchApp
            | Action::OpenInTerminal
            | Action::OpenInEditor(_)
            | Action::OpenClaudeCode(_)
            | Action::OpenCodex => draw_path_icon(draw.hDC, &entry.path, draw.rcItem, dpi),
            // `cc ` のフォルダ候補 (`claude_code_folder_entries`) は検索欄を
            // 補完する `ReplaceQuery` だが、実体はフォルダなので他の
            // フォルダ候補と同じアイコンにする。`az` のサブコマンド補完など
            // path を持たない `ReplaceQuery` は下のコマンドアイコンへ回す。
            Action::ReplaceQuery(_) if !entry.path.is_empty() => {
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
            | Action::KillProcess(_) => draw_command_icon(draw.hDC, draw.rcItem, dpi, name_font),
        }
    }
}

/// 行の右端に、種別タグ (選択行では Enter で起きること) と Git ブランチを
/// 右から順に描き、名前・詳細テキストが使える右端の x 座標を返す。
/// ブランチは旧描画では名前の後ろに `[main]` と連結していたが、
/// 長い名前だと省略記号で消えるため独立したタグにした。
unsafe fn draw_trailing_tags(
    draw: &DRAWITEMSTRUCT,
    entry: &Entry,
    badge: Option<&str>,
    selected: bool,
    dpi: u32,
    font: Option<HFONT>,
) -> i32 {
    let text_right = draw.rcItem.right - scale(TAG_RIGHT, dpi);
    let Some(font) = font else {
        return text_right;
    };
    let row_bg = if selected { SELECTED_BG } else { BACKGROUND };
    unsafe {
        let (label, style) = if selected {
            (
                action_verb(&entry.action),
                TagStyle {
                    fill: row_bg,
                    border: row_bg,
                    text: SELECTED_TEXT_SECONDARY,
                },
            )
        } else {
            (
                action_tag(&entry.action, badge, &entry.path),
                TagStyle {
                    fill: TAG_BG,
                    border: TAG_BG,
                    text: action_tag_color(&entry.action),
                },
            )
        };
        let mut left =
            draw_tag_right_aligned(draw.hDC, label, text_right, draw.rcItem, dpi, font, style);
        if let Some(branch) = entry.branch.as_deref() {
            let style = if selected {
                TagStyle {
                    fill: row_bg,
                    border: SELECTED_TAG_BORDER,
                    text: SELECTED_TEXT_SECONDARY,
                }
            } else {
                TagStyle {
                    fill: row_bg,
                    border: BRANCH_BORDER,
                    text: BRANCH_TEXT,
                }
            };
            left = draw_tag_right_aligned(
                draw.hDC,
                branch,
                left - scale(8, dpi),
                draw.rcItem,
                dpi,
                font,
                style,
            );
        }
        left - scale(12, dpi)
    }
}

pub(super) fn detail_primary(entry: &Entry) -> String {
    if entry.breadcrumb.is_empty() {
        entry.path.clone()
    } else {
        entry.breadcrumb.clone()
    }
}

/// breadcrumb の右側に淡色で続けるパス (中黒で区切る)。breadcrumb が無い、または
/// パスと同じ内容を primary で既に出している場合は None。
/// Azure DevOps 項目は breadcrumb 自体に作成者などの有益情報を持つため、
/// 冗長でしかない URL は表示しない。
pub(super) fn detail_secondary(entry: &Entry) -> Option<String> {
    (!entry.breadcrumb.is_empty()
        && !entry.path.is_empty()
        && !entry.breadcrumb.starts_with("Azure DevOps —"))
    .then(|| format!("  \u{00B7}  {}", entry.path))
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
