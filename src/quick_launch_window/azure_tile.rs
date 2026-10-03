//! Azure DevOps 候補の行頭アイコン (種別色の角丸タイル + 白い記号 + 状態の点)。
//!
//! GDI の `RoundRect` / `Ellipse` はアンチエイリアスが効かず、22px 程度の
//! タイルでは縁がギザギザになる。GDI+ も持ち込んでいないため、形状は
//! 4x4 のスーパーサンプリングで乗算済み BGRA のピクセル列を作り、
//! `AlphaBlend` で重ねる。記号は従来どおり `DrawText` で描く。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HDC, HFONT,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use super::draw::draw_text_centered;
use super::layout::scale;
use super::{ICON_LEFT, ICON_SIZE, rgb};

/// 1 ピクセルあたりの縦横サンプル数。
const SAMPLES: usize = 4;

/// 完了済み候補のタイルの不透明度 (0〜255)。行を読まずに「もう終わった
/// もの」と分かる程度に沈める。
const CLOSED_ALPHA: u8 = 110;

/// `sample(x, y)` が返す色でピクセルを塗る。`None` は透明。
/// 戻り値は `size * size` 個の乗算済み BGRA (上から下)。
fn render(size: usize, sample: impl Fn(f32, f32) -> Option<COLORREF>) -> Vec<u32> {
    let step = 1.0 / SAMPLES as f32;
    let total = (SAMPLES * SAMPLES) as u32;
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let (mut red, mut green, mut blue, mut alpha) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) * step;
                    let py = y as f32 + (sy as f32 + 0.5) * step;
                    if let Some(color) = sample(px, py) {
                        red += color.0 & 0xff;
                        green += (color.0 >> 8) & 0xff;
                        blue += (color.0 >> 16) & 0xff;
                        alpha += 255;
                    }
                }
            }
            // 各チャネルは「覆われたサンプルの色の和 / 全サンプル数」で、
            // そのまま乗算済みアルファの値になる。
            pixels.push(
                (blue / total)
                    | ((green / total) << 8)
                    | ((red / total) << 16)
                    | ((alpha / total) << 24),
            );
        }
    }
    pixels
}

/// 半径 `radius` の角丸四角 (`0..size` 四方) に点が含まれるか。
fn in_rounded_rect(x: f32, y: f32, size: f32, radius: f32) -> bool {
    if !(0.0..size).contains(&x) || !(0.0..size).contains(&y) {
        return false;
    }
    let cx = x.clamp(radius, size - radius);
    let cy = y.clamp(radius, size - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// 種別色の角丸タイル。
pub(super) fn tile_pixels(size: usize, color: COLORREF) -> Vec<u32> {
    let side = size as f32;
    let radius = side * 0.22;
    render(size, |x, y| {
        in_rounded_rect(x, y, side, radius).then_some(color)
    })
}

/// 右下の状態の点。行の地の色 `ring` で縁取り、タイルとの境目を作る。
pub(super) fn marker_pixels(size: usize, color: COLORREF, ring: COLORREF) -> Vec<u32> {
    let side = size as f32;
    let dot = side * 0.35 / 2.0;
    let outer = dot + (side * 0.09).max(1.0);
    let center = side - outer;
    render(size, |x, y| {
        let distance = (x - center).powi(2) + (y - center).powi(2);
        if distance <= dot * dot {
            Some(color)
        } else if distance <= outer * outer {
            Some(ring)
        } else {
            None
        }
    })
}

/// 乗算済み BGRA を `rect` の行頭アイコン位置へ `alpha` の不透明度で重ねる。
unsafe fn blend_pixels(hdc: HDC, pixels: &[u32], size: i32, left: i32, top: i32, alpha: u8) {
    unsafe {
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(hdc), &header, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            return;
        };
        std::slice::from_raw_parts_mut(bits.cast::<u32>(), pixels.len()).copy_from_slice(pixels);
        let source = CreateCompatibleDC(Some(hdc));
        if !source.is_invalid() {
            let old = SelectObject(source, bitmap.into());
            let _ = AlphaBlend(
                hdc,
                left,
                top,
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
                    SourceConstantAlpha: alpha,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                },
            );
            SelectObject(source, old);
            let _ = DeleteDC(source);
        }
        let _ = DeleteObject(bitmap.into());
    }
}

/// 描くアイコンの中身。色と記号は `badge.rs` が種別・状態から決める。
pub(super) struct AzureTile {
    pub(super) color: COLORREF,
    pub(super) glyph: std::borrow::Cow<'static, str>,
    pub(super) marker: Option<COLORREF>,
    pub(super) closed: bool,
}

/// `row_bg` は行の地の色 (選択行なら選択色)。点の縁取りと、完了済み候補の
/// 記号を沈める先の色に使う。
pub(super) unsafe fn draw_azure_tile(
    hdc: HDC,
    tile: &AzureTile,
    rect: RECT,
    dpi: u32,
    font: Option<HFONT>,
    row_bg: COLORREF,
) {
    let size = scale(ICON_SIZE, dpi);
    let left = rect.left + scale(ICON_LEFT, dpi);
    let top = rect.top + (rect.bottom - rect.top - size) / 2;
    let alpha = if tile.closed { CLOSED_ALPHA } else { 255 };
    unsafe {
        blend_pixels(
            hdc,
            &tile_pixels(size as usize, tile.color),
            size,
            left,
            top,
            alpha,
        );
        if let Some(font) = font {
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(
                hdc,
                if tile.closed {
                    mix(rgb(255, 255, 255), row_bg, alpha)
                } else {
                    rgb(255, 255, 255)
                },
            );
            let mut glyph_rect = RECT {
                left,
                top,
                right: left + size,
                bottom: top + size,
            };
            draw_text_centered(hdc, &tile.glyph, &mut glyph_rect);
            SelectObject(hdc, old_font);
        }
        if let Some(marker) = tile.marker {
            let pixels = marker_pixels(size as usize, marker, row_bg);
            blend_pixels(hdc, &pixels, size, left, top, 255);
        }
    }
}

/// `front` を不透明度 `alpha` で `back` に重ねた色。
fn mix(front: COLORREF, back: COLORREF, alpha: u8) -> COLORREF {
    let channel = |shift: u32| -> u8 {
        let f = (front.0 >> shift) & 0xff;
        let b = (back.0 >> shift) & 0xff;
        ((f * u32::from(alpha) + b * (255 - u32::from(alpha))) / 255) as u8
    };
    rgb(channel(0), channel(8), channel(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixel: u32) -> u32 {
        pixel >> 24
    }

    #[test]
    fn tile_is_opaque_in_the_middle_and_clear_at_the_corners() {
        let size = 22;
        let pixels = tile_pixels(size, rgb(191, 90, 242));
        let center = pixels[size / 2 * size + size / 2];
        assert_eq!(alpha(center), 255);
        // 乗算済み BGRA: 不透明なら色がそのまま入る
        assert_eq!(center & 0xff, 242);
        assert_eq!((center >> 16) & 0xff, 191);
        assert_eq!(alpha(pixels[0]), 0);
        assert_eq!(alpha(pixels[size * size - 1]), 0);
    }

    #[test]
    fn tile_edges_are_antialiased() {
        let size = 22;
        let pixels = tile_pixels(size, rgb(255, 255, 255));
        // 角丸の弧にかかるピクセルは半透明になる (ギザギザにならない)
        assert!(pixels.iter().any(|pixel| (1..255).contains(&alpha(*pixel))));
    }

    #[test]
    fn marker_sits_in_the_bottom_right_with_a_ring() {
        let size = 22;
        let dot = rgb(255, 149, 0);
        let ring = rgb(17, 17, 17);
        let pixels = marker_pixels(size, dot, ring);
        assert_eq!(alpha(pixels[0]), 0, "左上には何も描かない");
        assert_eq!(
            alpha(pixels[size / 2 * size + size / 2]),
            0,
            "中央の記号にかぶせない"
        );
        let side = size as f32;
        let center = (side - side * 0.35 / 2.0 - side * 0.09) as usize;
        let pixel = pixels[center * size + center];
        assert_eq!(alpha(pixel), 255);
        assert_eq!((pixel >> 16) & 0xff, 255, "点の中心は点の色");
    }

    #[test]
    fn mix_blends_toward_the_background() {
        assert_eq!(
            mix(rgb(255, 255, 255), rgb(0, 0, 0), 255),
            rgb(255, 255, 255)
        );
        assert_eq!(mix(rgb(255, 255, 255), rgb(0, 0, 0), 0), rgb(0, 0, 0));
    }
}
