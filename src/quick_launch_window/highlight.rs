//! 候補一覧の名前欄で、検索語に一致した箇所だけ色を変えて描く。
//!
//! `quick_launch::highlight_ranges` が返すバイト範囲を元に、通常テキストと
//! 一致テキストを交互に `DrawTextW` するだけ (GDI に部分着色 API は無い)。
//! 一致箇所は色に加えて下線付き太字 (`State::highlight_font`) で描く。
//! 選択行では名前と一致箇所の色差が小さくなり、色だけでは区別できないため。
//! `DT_END_ELLIPSIS` の自動省略は複数回描画では再現できないため、全体が
//! ボックス幅に収まらない場合はハイライトなしの単色 + 省略記号描画へ
//! フォールバックする。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC, HFONT, SelectObject,
    SetTextColor,
};

use super::draw::draw_text;

/// `text` を `ranges` (バイト範囲、`text` に対して昇順・非重複) に従って
/// 塗り分けて描く。範囲が空、または全体がボックス幅に収まらない場合は
/// 単色 + 省略記号の通常描画にフォールバックする。
/// `highlight_font` は一致箇所だけに使うフォント (None なら現在のフォントのまま)。
pub(super) unsafe fn draw_text_highlighted(
    hdc: HDC,
    text: &str,
    ranges: &[(usize, usize)],
    rect: &mut RECT,
    normal_color: COLORREF,
    highlight_color: COLORREF,
    highlight_font: Option<HFONT>,
) {
    unsafe {
        if ranges.is_empty() {
            SetTextColor(hdc, normal_color);
            draw_text(hdc, text, rect);
            return;
        }
        let mut segments = Vec::with_capacity(ranges.len() * 2 + 1);
        let mut pos = 0usize;
        for &(start, end) in ranges {
            if start > pos {
                segments.push((&text[pos..start], false));
            }
            segments.push((&text[start..end], true));
            pos = end;
        }
        if pos < text.len() {
            segments.push((&text[pos..], false));
        }
        // 一致箇所は太字で幅が変わるため、区間ごとのフォントで測った合計で判定する
        let widths: Vec<i32> = segments
            .iter()
            .map(|&(segment, highlighted)| {
                let font = if highlighted { highlight_font } else { None };
                with_font(hdc, font, || full_width(hdc, segment, rect))
            })
            .collect();
        if widths.iter().sum::<i32>() > rect.right - rect.left {
            SetTextColor(hdc, normal_color);
            draw_text(hdc, text, rect);
            return;
        }
        let mut x = rect.left;
        for (&(segment, highlighted), width) in segments.iter().zip(widths) {
            let (color, font) = if highlighted {
                (highlight_color, highlight_font)
            } else {
                (normal_color, None)
            };
            with_font(hdc, font, || {
                draw_segment(hdc, segment, x, width, rect, color)
            });
            x += width;
        }
    }
}

/// `font` が Some の間だけそれを選択して `body` を実行し、元のフォントへ戻す。
unsafe fn with_font<T>(hdc: HDC, font: Option<HFONT>, body: impl FnOnce() -> T) -> T {
    let Some(font) = font else {
        return body();
    };
    unsafe {
        let old = SelectObject(hdc, font.into());
        let result = body();
        SelectObject(hdc, old);
        result
    }
}

/// `text` を単色で `x` から `width` 幅に描く。空文字列は何もしない。
unsafe fn draw_segment(hdc: HDC, text: &str, x: i32, width: i32, rect: &RECT, color: COLORREF) {
    if text.is_empty() {
        return;
    }
    unsafe {
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
