//! 候補一覧の名前欄で、検索語に一致した箇所だけ色を変えて描く。
//!
//! `quick_launch::highlight_ranges` が返すバイト範囲を元に、通常テキストと
//! 一致テキストを交互に `DrawTextW` するだけ (GDI に部分着色 API は無い)。
//! `DT_END_ELLIPSIS` の自動省略は複数回描画では再現できないため、全体が
//! ボックス幅に収まらない場合はハイライトなしの単色 + 省略記号描画へ
//! フォールバックする。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC, SetTextColor,
};

use super::draw::draw_text;

/// `text` を `ranges` (バイト範囲、`text` に対して昇順・非重複) に従って
/// 塗り分けて描く。範囲が空、または全体がボックス幅に収まらない場合は
/// 単色 + 省略記号の通常描画にフォールバックする。
pub(super) unsafe fn draw_text_highlighted(
    hdc: HDC,
    text: &str,
    ranges: &[(usize, usize)],
    rect: &mut RECT,
    normal_color: COLORREF,
    highlight_color: COLORREF,
) {
    unsafe {
        if ranges.is_empty() || full_width(hdc, text, rect) > rect.right - rect.left {
            SetTextColor(hdc, normal_color);
            draw_text(hdc, text, rect);
            return;
        }
        let mut x = rect.left;
        let mut pos = 0usize;
        for &(start, end) in ranges {
            if start > pos {
                x = draw_segment(hdc, &text[pos..start], x, rect, normal_color);
            }
            x = draw_segment(hdc, &text[start..end], x, rect, highlight_color);
            pos = end;
        }
        if pos < text.len() {
            draw_segment(hdc, &text[pos..], x, rect, normal_color);
        }
    }
}

/// `text` を単色で描き、描画後の右端 x 座標を返す。空文字列は何もしない。
unsafe fn draw_segment(hdc: HDC, text: &str, x: i32, rect: &RECT, color: COLORREF) -> i32 {
    if text.is_empty() {
        return x;
    }
    unsafe {
        let width = full_width(hdc, text, rect);
        SetTextColor(hdc, color);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut seg_rect = RECT {
            left: x,
            top: rect.top,
            right: x + width,
            bottom: rect.bottom,
        };
        DrawTextW(
            hdc,
            &mut wide,
            &mut seg_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        x + width
    }
}

/// 省略記号なしで `text` を描いたときの実幅を測る (`DT_CALCRECT`、実際には描画しない)。
unsafe fn full_width(hdc: HDC, text: &str, rect: &RECT) -> i32 {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut calc_rect = *rect;
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            &mut calc_rect,
            DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT,
        );
    }
    calc_rect.right - calc_rect.left
}
