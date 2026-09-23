//! 候補行のアイコン描画（Azure種別・パス/ウィンドウ/favicon）。

use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, CreateCompatibleDC, DeleteDC, HBITMAP,
    HDC, HFONT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use super::badge::{AzureIconKind, azure_icon_label};
use super::draw::draw_text_centered;
use super::layout::scale;
use super::{ACCENT, ICON_LEFT, ICON_SIZE};

/// 種別色をそのまま塗ると強すぎるので、背景 (`BACKGROUND`) に大きく
/// 寄せた低彩度版にする (モードバッジの地に使う)。
pub(super) fn backdrop_tint(color: COLORREF) -> COLORREF {
    let mix = |channel: u8| -> u8 { ((channel as u32 * 46 + 13 * 210) / 256) as u8 };
    super::rgb(
        mix((color.0 & 0xff) as u8),
        mix(((color.0 >> 8) & 0xff) as u8),
        mix(((color.0 >> 16) & 0xff) as u8),
    )
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

/// シェル名前空間の指定かどうか。`shell:MyComputerFolder` と
/// `::{GUID}` はファイルパスではないため、ファイル扱いで引くと必ず失敗する
/// (実測: `::{20D04FE0-...}` は `SHGetFileInfo` が何も返さない)。
pub(super) fn is_shell_namespace(path: &str) -> bool {
    path.starts_with("shell:") || path.starts_with("::{")
}

pub(super) unsafe fn draw_path_icon(hdc: HDC, path: &str, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    let bitmap = if is_shell_namespace(path) {
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

/// favicon が無いブックマーク／履歴に使う既定アイコン (星マーク)。
const ICON_BOOKMARK: &[u8] = include_bytes!("../../assets/menu/bookmark.png");

/// favicon が無い開いているタブに使う既定アイコン (ブラウザタブ形状)。
const ICON_TAB: &[u8] = include_bytes!("../../assets/menu/tab.png");

/// favicon が見つからないときのフォールバック種別。
#[derive(Clone, Copy)]
pub(super) enum FaviconFallback {
    Bookmark,
    Tab,
    /// Web 検索 (`??`、FR-9.21)。検索エンジンの favicon が
    /// ブラウザの DB にあればそれを使う。
    Web,
}

/// URL 系候補の favicon を描く。Chrome/Edge の `Favicons` DB に
/// 見つからなければ `fallback` に応じた既定アイコンへフォールバックする。
pub(super) unsafe fn draw_favicon_icon(
    hdc: HDC,
    url: &str,
    rect: RECT,
    dpi: u32,
    fallback: FaviconFallback,
) {
    let size = scale(ICON_SIZE, dpi);
    let bitmap = crate::icon::bitmap_for_favicon_sized(url, size).or_else(|| match fallback {
        FaviconFallback::Bookmark => {
            crate::icon::bitmap_for_asset_sized("bookmark", ICON_BOOKMARK, size)
        }
        FaviconFallback::Tab => crate::icon::bitmap_for_asset_sized("tab", ICON_TAB, size),
        // 検索エンジンの favicon がブラウザの DB に無いときの代替。
        // Web 検索専用のアセットは持たず、URL を開く点で最も近い
        // ブックマークのアイコンを共用する
        FaviconFallback::Web => {
            crate::icon::bitmap_for_asset_sized("bookmark", ICON_BOOKMARK, size)
        }
    });
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

#[cfg(test)]
mod tests {
    use super::is_shell_namespace;

    #[test]
    fn detects_both_spellings_of_shell_namespace() {
        assert!(is_shell_namespace("shell:MyComputerFolder"));
        assert!(is_shell_namespace(
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"
        ));
        assert!(!is_shell_namespace(r"C:\Users"));
        assert!(!is_shell_namespace("https://example.com/"));
    }
}
