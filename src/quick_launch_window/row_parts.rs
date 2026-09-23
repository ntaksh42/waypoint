//! 候補リストの行を構成する部品 (セクション見出し・右端のタグ)。
//!
//! `draw_row.rs` が 1 行ぶんの組み立てを受け持ち、ここは見出し行と
//! 角丸タグ (種別・ブランチ・実行操作、操作ヒント帯のキー表記) の描画だけを置く。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, HDC, HFONT, PS_SOLID, RoundRect,
    SelectObject, SetBkMode, SetTextCharacterExtra, SetTextColor, TRANSPARENT,
};

use super::draw::{draw_text, draw_text_centered, measured_width};
use super::layout::scale;
use super::{HEADER_RULE, HEADER_TEXT, ICON_LEFT, TAG_RIGHT, TEXT_MUTED};

/// タグの配色。`fill` と `border` を行の地と同じ色にすると枠の無い文字だけになる。
#[derive(Clone, Copy)]
pub(super) struct TagStyle {
    pub(super) fill: COLORREF,
    pub(super) border: COLORREF,
    pub(super) text: COLORREF,
}

/// 絞り込みなし一覧の区分見出し行を描く。大文字・字間広めのラベルに件数を
/// 添え、残りの幅に細い区切り線を引く。本文の補助テキストと同じ見た目だった
/// 旧描画では、見出しが項目行に埋もれて区切りとして機能していなかった。
pub(super) unsafe fn draw_section_header(
    hdc: HDC,
    label: &str,
    count: usize,
    rect: RECT,
    dpi: u32,
    header_font: Option<HFONT>,
    count_font: Option<HFONT>,
) {
    let Some(header_font) = header_font else {
        return;
    };
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        // 見出しは次の項目行に寄せ、上側を広く空けて区分の切れ目を作る
        let mut text_rect = RECT {
            left: rect.left + scale(ICON_LEFT, dpi),
            top: rect.top + scale(12, dpi),
            right: rect.right - scale(TAG_RIGHT, dpi),
            bottom: rect.bottom - scale(4, dpi),
        };
        let label = label.to_uppercase();
        let old_font = SelectObject(hdc, header_font.into());
        let old_extra = SetTextCharacterExtra(hdc, scale(1, dpi));
        SetTextColor(hdc, HEADER_TEXT);
        let label_width = measured_width(hdc, &label, &text_rect);
        draw_text(hdc, &label, &mut text_rect);
        SetTextCharacterExtra(hdc, old_extra);
        SelectObject(hdc, old_font);

        let mut x = text_rect.left + label_width + scale(8, dpi);
        if let Some(font) = count_font {
            let old_font = SelectObject(hdc, font.into());
            SetTextColor(hdc, TEXT_MUTED);
            let count = count.to_string();
            let mut count_rect = RECT {
                left: x,
                ..text_rect
            };
            let count_width = measured_width(hdc, &count, &count_rect);
            draw_text(hdc, &count, &mut count_rect);
            SelectObject(hdc, old_font);
            x += count_width + scale(8, dpi);
        }

        let middle = (text_rect.top + text_rect.bottom) / 2;
        let rule = RECT {
            left: x.min(text_rect.right),
            top: middle,
            right: text_rect.right,
            bottom: middle + scale(1, dpi).max(1),
        };
        let brush = CreateSolidBrush(HEADER_RULE);
        FillRect(hdc, &rule, brush);
        let _ = DeleteObject(brush.into());
    }
}

/// `label` をタグとして描いたときの幅 (左右の余白込み)。`font` は選択済みの前提。
pub(super) unsafe fn tag_width(hdc: HDC, label: &str, bounds: &RECT, dpi: u32) -> i32 {
    unsafe { measured_width(hdc, label, bounds) + scale(7, dpi) * 2 }
}

/// 角丸タグを `left` から描き、右端の x 座標を返す。高さは 20px で
/// `row` の縦中央に置く。
pub(super) unsafe fn draw_tag(
    hdc: HDC,
    label: &str,
    left: i32,
    row: RECT,
    dpi: u32,
    font: HFONT,
    style: TagStyle,
) -> i32 {
    unsafe {
        let old_font = SelectObject(hdc, font.into());
        let width = tag_width(hdc, label, &row, dpi);
        let height = scale(20, dpi);
        let top = row.top + (row.bottom - row.top - height) / 2;
        let mut rect = RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        let brush = CreateSolidBrush(style.fill);
        let pen = CreatePen(PS_SOLID, 1, style.border);
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let radius = scale(8, dpi);
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

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, style.text);
        draw_text_centered(hdc, label, &mut rect);
        SelectObject(hdc, old_font);
        rect.right
    }
}

/// 角丸タグを右端 `right` に揃えて描き、左端の x 座標を返す。
pub(super) unsafe fn draw_tag_right_aligned(
    hdc: HDC,
    label: &str,
    right: i32,
    row: RECT,
    dpi: u32,
    font: HFONT,
    style: TagStyle,
) -> i32 {
    unsafe {
        let old_font = SelectObject(hdc, font.into());
        let width = tag_width(hdc, label, &row, dpi);
        SelectObject(hdc, old_font);
        let left = right - width;
        draw_tag(hdc, label, left, row, dpi, font, style);
        left
    }
}
