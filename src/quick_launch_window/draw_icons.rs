//! 候補行のアイコン描画（種別背景・Azure種別・パス/ウィンドウ/favicon）。

use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, CreateCompatibleDC, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, Ellipse, HBITMAP, HDC, HFONT, PS_SOLID, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};

use super::badge::{AzureIconKind, azure_icon_label};
use super::draw::draw_text_centered;
use super::layout::scale;
use super::{ACCENT, ICON_LEFT, ICON_SIZE};

/// 種別色をそのまま塗ると強すぎるので、背景 (`BACKGROUND`) に大きく
/// 寄せた低彩度版にする。
pub(super) fn backdrop_tint(color: COLORREF) -> COLORREF {
    let mix = |channel: u8| -> u8 { ((channel as u32 * 46 + 13 * 210) / 256) as u8 };
    super::rgb(
        mix((color.0 & 0xff) as u8),
        mix(((color.0 >> 8) & 0xff) as u8),
        mix(((color.0 >> 16) & 0xff) as u8),
    )
}

/// 種別色の丸背景。実アイコンより一回り大きく敷き、透明な余白を持つ
/// アイコン (フォルダ・URL 等) でも種別が一目で分かるようにする。
pub(super) unsafe fn draw_icon_backdrop(hdc: HDC, color: COLORREF, rect: RECT, dpi: u32) {
    unsafe {
        let size = scale(ICON_SIZE + 6, dpi);
        let left = rect.left + scale(ICON_LEFT, dpi) - scale(3, dpi);
        let top = rect.top + (rect.bottom - rect.top - size) / 2;
        let brush = CreateSolidBrush(backdrop_tint(color));
        let pen = CreatePen(PS_SOLID, 1, backdrop_tint(color));
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = Ellipse(hdc, left, top, left + size, top + size);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

/// Azure DevOps の種別を小さなグリフとして描く。外部アイコンの読込を
/// 増やさず、PR / WIT / Pipeline / Project を色と形で区別する。
pub(super) unsafe fn draw_azure_icon(
    hdc: HDC,
    kind: AzureIconKind,
    color: COLORREF,
    rect: RECT,
    dpi: u32,
    font: Option<HFONT>,
) {
    let Some(font) = font else { return };
    unsafe {
        let size = scale(ICON_SIZE, dpi);
        let mut icon_rect = RECT {
            left: rect.left + scale(ICON_LEFT, dpi),
            top: rect.top + (rect.bottom - rect.top - size) / 2,
            right: rect.left + scale(ICON_LEFT, dpi) + size,
            bottom: rect.top + (rect.bottom - rect.top - size) / 2 + size,
        };
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        draw_text_centered(hdc, azure_icon_label(kind), &mut icon_rect);
        SelectObject(hdc, old_font);
    }
}

/// コマンド候補は URL やファイルの実体を持たないため、補完操作であることを
/// 小さな `>` グリフで示す。
pub(super) unsafe fn draw_command_icon(hdc: HDC, rect: RECT, dpi: u32, font: Option<HFONT>) {
    let Some(font) = font else { return };
    unsafe {
        let size = scale(ICON_SIZE, dpi);
        let mut icon_rect = RECT {
            left: rect.left + scale(ICON_LEFT, dpi),
            top: rect.top + (rect.bottom - rect.top - size) / 2,
            right: rect.left + scale(ICON_LEFT, dpi) + size,
            bottom: rect.top + (rect.bottom - rect.top - size) / 2 + size,
        };
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, ACCENT);
        draw_text_centered(hdc, ">", &mut icon_rect);
        SelectObject(hdc, old_font);
    }
}

pub(super) unsafe fn draw_path_icon(hdc: HDC, path: &str, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    // shell:MyComputerFolder 等はファイルパスではないため専用の解決経路を使う
    let bitmap = if path.starts_with("shell:") {
        crate::icon::bitmap_for_shell_sized(path, size)
    } else {
        crate::icon::bitmap_for_sized(path, size)
    };
    let Some(bitmap) = bitmap else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

pub(super) unsafe fn draw_window_icon(hdc: HDC, hwnd: HWND, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    let Some(bitmap) = crate::icon::bitmap_for_window_sized(hwnd, size) else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

/// favicon が無いブックマークに使う既定アイコン (星マーク)。
const ICON_BOOKMARK: &[u8] = include_bytes!("../../assets/menu/bookmark.png");

/// ブックマークの favicon を描く。Chrome/Edge の `Favicons` DB に
/// 見つからなければ既定の星マークへフォールバックする。
pub(super) unsafe fn draw_favicon_icon(hdc: HDC, url: &str, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    let bitmap = crate::icon::bitmap_for_favicon_sized(url, size)
        .or_else(|| crate::icon::bitmap_for_asset_sized("bookmark", ICON_BOOKMARK, size));
    let Some(bitmap) = bitmap else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

/// `bitmap` は既に `size` 四方で生成済みの前提で等倍コピーする。
///
/// 寸法違いのビットマップを AlphaBlend で拡大縮小すると輪郭がにじむため、
/// 呼び出し側 (`icon::bitmap_for_sized` / `bitmap_for_window_sized`) で
/// 要求寸法どおりのビットマップを取得させ、ここでは伸縮させない。
pub(super) unsafe fn draw_icon_bitmap(hdc: HDC, bitmap: HBITMAP, rect: RECT, dpi: u32, size: i32) {
    unsafe {
        let source = CreateCompatibleDC(Some(hdc));
        if source.is_invalid() {
            return;
        }
        let old = SelectObject(source, bitmap.into());
        let _ = AlphaBlend(
            hdc,
            rect.left + scale(ICON_LEFT, dpi),
            rect.top + (rect.bottom - rect.top - size) / 2,
            size,
            size,
            source,
            0,
            0,
            size,
            size,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            },
        );
        SelectObject(source, old);
        let _ = DeleteDC(source);
    }
}
