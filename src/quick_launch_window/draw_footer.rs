//! ウィンドウ下端の操作ヒント帯。
//!
//! 選択中の候補に対して使えるキー操作 (FR-9) は、知らなければ
//! 使われないまま埋もれる。キーボードだけで完結させる方針に沿って、
//! 選択中の候補で実際に効く操作だけを下端へ並べる。固定の 4 操作を
//! 常に出していた旧表示では、ウィンドウ候補にも効かない `Ctrl+E` 等が並び、
//! 逆に `Ctrl+W` (ウィンドウを閉じる) のような候補固有の操作は出なかった。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, FillRect, HDC,
    HFONT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use super::badge::action_verb;
use super::draw::{draw_text, measured_width};
use super::layout::scale;
use super::row_parts::{TagStyle, draw_tag};
use super::{
    FOOTER_BG, FOOTER_HEIGHT, FOOTER_RULE, FOOTER_TEXT, KEYCAP_BORDER, KEYCAP_TEXT, STATE,
};
use crate::quick_launch::{Action, Entry};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

const ENTER: &str = "\u{21B5}";
const COPY_PATH: (&str, &str) = ("Ctrl+C", "Copy path");
const REVEAL: (&str, &str) = ("Ctrl+E", "Reveal");
const FAVORITE: (&str, &str) = ("Ctrl+Shift+\u{21B5}", "Favorite");

const LIVE_SEARCH: (&str, &str) = ("Ctrl+\u{21B5}", "Live search");

/// 選択中の候補 `entry` で使えるキー操作を、左から並べる順に返す。
/// 先頭は必ず Enter (選択行の右端に出す動詞と同じ語)。候補が無ければ空。
/// `live` は現在の入力で `Ctrl+Enter` の Azure Live 検索が成立するとき (FR-9.18.7)。
/// 候補が無い (0 件・検索中) 間も、次の一手として出す。
pub(super) fn footer_hints(entry: Option<&Entry>, live: bool) -> Vec<(&'static str, &'static str)> {
    let Some(entry) = entry else {
        return if live { vec![LIVE_SEARCH] } else { Vec::new() };
    };
    let verb = action_verb(&entry.action)
        .trim_end_matches(ENTER)
        .trim_end();
    let mut hints = vec![(ENTER, verb)];
    if live {
        hints.push(LIVE_SEARCH);
    }
    let has_path = !entry.path.is_empty();
    match &entry.action {
        Action::FocusWindow(_) => hints.push(("Ctrl+W", "Close window")),
        Action::FocusBrowserTab(_) | Action::OpenUrl(_) => {
            if has_path {
                hints.push(("Ctrl+C", "Copy URL"));
            }
            if entry.branch.is_some() {
                hints.push(("Ctrl+Shift+C", "Copy branch"));
            }
        }
        Action::OpenFolder(_) => {
            hints.extend([
                ("Shift+\u{21B5}", "New window"),
                REVEAL,
                COPY_PATH,
                FAVORITE,
            ]);
        }
        Action::LaunchApp | Action::OpenWithDefaultHandler => {
            hints.extend([
                ("Ctrl+Alt+\u{21B5}", "As admin"),
                REVEAL,
                COPY_PATH,
                FAVORITE,
            ]);
        }
        Action::OpenInTerminal
        | Action::OpenInEditor(_)
        | Action::OpenClaudeCode(_)
        | Action::OpenCodex
        | Action::ResumeAgentSession(..)
        | Action::ReplaceQuery(_)
            if has_path =>
        {
            hints.extend([REVEAL, COPY_PATH]);
        }
        _ => {}
    }
    hints
}

/// 右端に出す位置表示 ("3 / 24")。`selected` は項目行の中での 0 始まりの順位。
pub(super) fn footer_position(selected: Option<usize>, total: usize) -> Option<String> {
    (total > 0).then(|| match selected {
        Some(index) => format!("{} / {}", index + 1, total),
        None => format!("{total} results"),
    })
}

/// 下端の操作ヒント帯だけを再描画させる。ヒントと位置表示は選択中の
/// 候補に従うため、選択が動くたびに呼ぶ。STATE を借用中に呼ばないこと。
pub(super) fn invalidate_footer() {
    let (window, dpi) = STATE.with(|state| {
        let state = state.borrow();
        (state.window, state.dpi)
    });
    let Some(window) = window else {
        return;
    };
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        let footer_rect = RECT {
            top: client.bottom - scale(FOOTER_HEIGHT, dpi),
            ..client
        };
        let _ = InvalidateRect(Some(window), Some(&footer_rect), false);
    }
}

/// `client` の下端 `FOOTER_HEIGHT` に操作ヒント帯を描く。
pub(super) unsafe fn draw_footer(
    hdc: HDC,
    client: RECT,
    dpi: u32,
    font: Option<HFONT>,
    hints: &[(&str, &str)],
    position: Option<&str>,
) {
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
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, FOOTER_TEXT);
        // 位置表示を先に右端へ置き、ヒントはその手前で打ち切る
        let mut right = band.right - scale(16, dpi);
        if let Some(position) = position {
            let mut rect = RECT { right, ..band };
            let width = measured_width(hdc, position, &rect);
            let mut wide: Vec<u16> = position.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut wide,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT,
            );
            right -= width + scale(16, dpi);
        }
        SelectObject(hdc, old_font);

        let keycap = TagStyle {
            fill: FOOTER_BG,
            border: KEYCAP_BORDER,
            text: KEYCAP_TEXT,
        };
        let mut x = band.left + scale(14, dpi);
        for (key, label) in hints {
            let old_font = SelectObject(hdc, font.into());
            let key_width = super::row_parts::tag_width(hdc, key, &band, dpi);
            let label_width = measured_width(hdc, label, &band);
            SelectObject(hdc, old_font);
            // 収まらない操作は途中で切らずに丸ごと省く (後ろほど使用頻度が低い)
            if x + key_width + scale(6, dpi) + label_width > right {
                break;
            }
            x = draw_tag(hdc, key, x, band, dpi, font, keycap) + scale(6, dpi);
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, FOOTER_TEXT);
            let mut text_rect = RECT { left: x, ..band };
            draw_text(hdc, label, &mut text_rect);
            SelectObject(hdc, old_font);
            x += label_width + scale(16, dpi);
        }
    }
}
