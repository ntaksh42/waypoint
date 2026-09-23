//! ウィンドウ下端の操作ヒント帯。
//!
//! 選択中の候補に対して使えるキー操作 (FR-9) は、知らなければ
//! 使われないまま埋もれる。キーボードだけで完結させる方針に沿って、
//! 主要な操作を常に下端へ並べておく。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, HDC, HFONT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};

use super::draw::{draw_text, measured_width};
use super::layout::scale;
use super::row_parts::{TagStyle, draw_tag};
use super::{FOOTER_BG, FOOTER_HEIGHT, FOOTER_RULE, FOOTER_TEXT, KEYCAP_BORDER, KEYCAP_TEXT};

/// 表示するキーと説明。左から順に並べる。
const HINTS: [(&str, &str); 4] = [
    ("\u{21B5}", "Open"),
    ("Ctrl+E", "Reveal"),
    ("Ctrl+C", "Copy path"),
    ("Ctrl+Shift+\u{21B5}", "Favorite"),
];

/// `client` の下端 `FOOTER_HEIGHT` に操作ヒント帯を描く。
pub(super) unsafe fn draw_footer(hdc: HDC, client: RECT, dpi: u32, font: Option<HFONT>) {
    let band = RECT {
        left: client.left,
        top: client.bottom - scale(FOOTER_HEIGHT, dpi),
        right: client.right,
        bottom: client.bottom,
    };
    unsafe {
        let brush = CreateSolidBrush(FOOTER_BG);
        FillRect(hdc, &band, brush);
        let _ = DeleteObject(brush.into());
        let rule = RECT {
            bottom: band.top + scale(1, dpi).max(1),
            ..band
        };
        let brush = CreateSolidBrush(FOOTER_RULE);
        FillRect(hdc, &rule, brush);
        let _ = DeleteObject(brush.into());

        let Some(font) = font else {
            return;
        };
        let keycap = TagStyle {
            fill: FOOTER_BG,
            border: KEYCAP_BORDER,
            text: KEYCAP_TEXT,
        };
        let mut x = band.left + scale(20, dpi);
        for (key, label) in HINTS {
            x = draw_tag(hdc, key, x, band, dpi, font, keycap) + scale(6, dpi);
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, FOOTER_TEXT);
            let mut text_rect = RECT { left: x, ..band };
            let width = measured_width(hdc, label, &text_rect);
            draw_text(hdc, label, &mut text_rect);
            SelectObject(hdc, old_font);
            x += width + scale(18, dpi);
        }
    }
}
