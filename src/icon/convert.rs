//! HICON/HBITMAP 変換・リサイズ系のヘルパー。

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush,
    GetObjectW, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHGetImageList, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DestroyIcon, DrawIconEx, GetIconInfo, HICON, ICONINFO,
};
use windows::core::{HSTRING, PCWSTR};

use super::scale::image_list_for;

pub(crate) fn load_bitmap(path: &str, size: i32) -> Option<HBITMAP> {
    let icon = system_icon(path, size)?;
    let bmp = icon_to_bitmap(icon, SIZE { cx: size, cy: size });
    unsafe {
        let _ = DestroyIcon(icon);
    }
    bmp
}

/// シェルが返すアイコンを、描画寸法に見合う解像度で取得する。
/// 実体を引けないパス (不在・未接続ドライブ・届かない UNC は 2.1 秒
/// ブロックした末に失敗) は、拡張子から引く型アイコンへ落とす。
pub(crate) fn system_icon(path: &str, wanted: i32) -> Option<HICON> {
    if !path.starts_with(r"\\")
        && let Some(icon) = shell_file_icon(path, wanted, Default::default(), Default::default())
    {
        return Some(icon);
    }
    shell_file_icon(path, wanted, attributes_for(path), SHGFI_USEFILEATTRIBUTES)
}

/// 実体を持たないパスを `SHGFI_USEFILEATTRIBUTES` で引くときに渡す属性。
/// 拡張子があればファイル、なければフォルダとして扱う。
fn attributes_for(path: &str) -> FILE_FLAGS_AND_ATTRIBUTES {
    if std::path::Path::new(path).extension().is_some() {
        FILE_ATTRIBUTE_NORMAL
    } else {
        FILE_ATTRIBUTE_DIRECTORY
    }
}

fn shell_file_icon(
    path: &str,
    wanted: i32,
    attributes: FILE_FLAGS_AND_ATTRIBUTES,
    extra: windows::Win32::UI::Shell::SHGFI_FLAGS,
) -> Option<HICON> {
    unsafe {
        let wide = HSTRING::from(path);
        let mut info = SHFILEINFOW::default();

        // まずシステムイメージリストから引く。こちらのほうが
        // ハンドルを増やさずに済む
        let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | extra;
        let ok = SHGetFileInfoW(
            &wide,
            attributes,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        // 添字はリスト間で共通なので、欲しい寸法のリストから引き直す
        if ok != 0
            && let Ok(list) = SHGetImageList::<IImageList>(image_list_for(wanted))
            && let Ok(icon) = list.GetIcon(info.iIcon, ILD_TRANSPARENT.0)
        {
            return Some(icon);
        }

        // 取れなければ直接アイコンを要求する
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            &wide,
            attributes,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | icon_size_flag(wanted) | extra,
        );
        (ok != 0 && !info.hIcon.is_invalid()).then_some(info.hIcon)
    }
}

/// `SHGetFileInfo` に渡す寸法フラグ。16px を超えるなら大アイコンを要求する。
fn icon_size_flag(size: i32) -> windows::Win32::UI::Shell::SHGFI_FLAGS {
    if size <= 16 {
        SHGFI_SMALLICON
    } else {
        SHGFI_LARGEICON
    }
}

pub(crate) fn shell_icon(target: &str, size: i32) -> Option<HICON> {
    unsafe {
        let target = HSTRING::from(target);
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(&target, None, &mut pidl, 0, None).ok()?;

        let mut info = SHFILEINFOW::default();
        let flags = SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_SMALLICON;
        let ok = SHGetFileInfoW(
            PCWSTR(pidl.cast()),
            Default::default(),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        let icon = if ok != 0 {
            SHGetImageList::<IImageList>(image_list_for(size))
                .ok()
                .and_then(|list| list.GetIcon(info.iIcon, ILD_TRANSPARENT.0).ok())
        } else {
            None
        };
        CoTaskMemFree(Some(pidl.cast()));
        icon
    }
}

/// HICON をメニューが受け付ける 32bit ビットマップへ描き移す。
///
/// `DrawIconEx` は指定寸法へ単純にストレッチ描画するだけで、拡大縮小の
/// 補間は行わない (最近傍相当)。アイコンのネイティブ解像度と要求寸法が
/// 食い違う場合 (例: 32px のシステムイメージリストから 18px を要求) に
/// 直接ここへ渡すと輪郭がにじむ (実測で確認済み)。
///
/// まずネイティブ解像度のまま描画し、要求寸法と違うときだけ
/// `image` クレートの Lanczos3 で縮小する。縮小は `rgba_to_bitmap` が
/// 使うのと同じ高品質パスに揃え、二重の粗い補間を避ける。
pub(crate) fn icon_to_bitmap(icon: HICON, size: SIZE) -> Option<HBITMAP> {
    let native = native_icon_size(icon).unwrap_or(size);
    let raw = draw_icon_at(icon, native)?;
    if native.cx == size.cx && native.cy == size.cy {
        return Some(raw);
    }
    let resized = resize_bitmap(raw, native, size);
    unsafe {
        let _ = DeleteObject(raw.into());
    }
    resized
}

/// アイコンが実際に持つ解像度。`hbmColor` の実寸から読む。
fn native_icon_size(icon: HICON) -> Option<SIZE> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let mut bitmap = BITMAP::default();
        let read = GetObjectW(
            info.hbmColor.into(),
            size_of::<BITMAP>() as i32,
            Some(std::ptr::from_mut(&mut bitmap).cast()),
        );
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        (read > 0).then_some(SIZE {
            cx: bitmap.bmWidth,
            cy: bitmap.bmHeight,
        })
    }
}

/// 指定寸法のまま (拡縮なし) アイコンを 32bit DIB へ描く。
fn draw_icon_at(icon: HICON, size: SIZE) -> Option<HBITMAP> {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }

        // トップダウン (負の height) の 32bit DIB。アルファを保つ
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.cx,
                biHeight: -size.cy,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(hdc), &header, DIB_RGB_COLORS, &mut bits, None, 0);
        let Ok(bitmap) = bitmap else {
            let _ = DeleteDC(hdc);
            return None;
        };

        let old: HGDIOBJ = SelectObject(hdc, bitmap.into());
        let drawn = draw_icon(hdc, icon, size);
        if drawn {
            restore_missing_alpha(hdc, icon, size, bits.cast::<u8>());
        }
        SelectObject(hdc, old);
        let _ = DeleteDC(hdc);

        if drawn {
            Some(bitmap)
        } else {
            let _ = DeleteObject(bitmap.into());
            None
        }
    }
}

/// `hdc` に選択済みの DIB へアイコンを等倍で描き、GDI の描画を DIB の
/// ビットへ反映させる (バッチされた描画が残ったままビットを読まない)。
unsafe fn draw_icon(hdc: HDC, icon: HICON, size: SIZE) -> bool {
    unsafe {
        let drawn = DrawIconEx(hdc, 0, 0, icon, size.cx, size.cy, 0, None, DI_NORMAL);
        let _ = GdiFlush();
        drawn.is_ok()
    }
}

/// アルファチャネルを持たない旧形式のアイコンに、マスクからアルファを補う。
///
/// 24bit 以下のアイコン (色ビットマップ + AND マスク) を 32bit DIB へ
/// `DrawIconEx` すると、色は描かれるがアルファは 0 のまま残る。
/// 候補行はこれを `AlphaBlend` (`AC_SRC_ALPHA`) で重ねるため全面透明と
/// みなされ、アイコンが丸ごと消える。古いアプリの実行ファイルや
/// `WM_GETICON` が返すウィンドウアイコンで起きる。
///
/// 黒地と白地に描いた結果の差が透過部分を表すので、そこからアルファを
/// 復元する。黒地の結果はそのままプリマルチプライド済みの色になる。
///
/// `bits` は `hdc` に選択済みの DIB のビット。GDI が書き込む間は
/// スライスを握らないよう、描画の前後でその都度作り直す。
unsafe fn restore_missing_alpha(hdc: HDC, icon: HICON, size: SIZE, bits: *mut u8) {
    let len = size.cx as usize * size.cy as usize * 4;
    let on_black = {
        let pixels = unsafe { std::slice::from_raw_parts_mut(bits, len) };
        if has_alpha(pixels) {
            return;
        }
        let on_black = pixels.to_vec();
        pixels.fill(0xff);
        on_black
    };
    let redrawn = unsafe { draw_icon(hdc, icon, size) };
    let pixels = unsafe { std::slice::from_raw_parts_mut(bits, len) };
    if redrawn {
        alpha_from_backgrounds(&on_black, pixels);
    } else {
        pixels.copy_from_slice(&on_black);
    }
}

/// どこか 1 画素でもアルファを持っているか。
fn has_alpha(pixels: &[u8]) -> bool {
    pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0)
}

/// 黒地 (`on_black`) と白地 (`on_white`) に描いた BGRA から、
/// プリマルチプライド済みの BGRA を `on_white` へ書き戻す。
///
/// 不透明な画素は地の色に依らず同じ値になり、透明な画素は黒地で 0・
/// 白地で 255 になる。その差を透過度として読む。
fn alpha_from_backgrounds(on_black: &[u8], on_white: &mut [u8]) {
    for (black, white) in on_black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(on_white.as_chunks_mut::<4>().0)
    {
        let spread = black[..3]
            .iter()
            .zip(&white[..3])
            .map(|(dark, light)| light.saturating_sub(*dark))
            .max()
            .unwrap_or(0);
        let alpha = 255 - spread;
        // XOR で反転表示する画素は差が色と噛み合わないことがあるので、
        // プリマルチプライドの前提 (色 <= アルファ) を崩さないよう丸める
        for (target, source) in white[..3].iter_mut().zip(&black[..3]) {
            *target = (*source).min(alpha);
        }
        white[3] = alpha;
    }
}

/// 32bit DIB ビットマップを Lanczos3 で別寸法へ描き直す。
fn resize_bitmap(bitmap: HBITMAP, from: SIZE, to: SIZE) -> Option<HBITMAP> {
    unsafe {
        let mut info = BITMAP::default();
        let read = GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some(std::ptr::from_mut(&mut info).cast()),
        );
        if read <= 0 || info.bmBits.is_null() {
            return None;
        }
        // DrawIconEx が書いた DIB はトップダウン (biHeight < 0) の
        // BGRA・プリマルチプライド済み。image クレートは非プリマルチプライドの
        // RGBA を期待するので、リサイズ前に一度戻す
        let stride = (from.cx as usize) * 4;
        let pixels =
            std::slice::from_raw_parts(info.bmBits.cast::<u8>(), stride * from.cy as usize);
        let mut rgba = vec![0u8; pixels.len()];
        for (source, target) in pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(rgba.as_chunks_mut::<4>().0)
        {
            let alpha = source[3];
            let unmultiply = |value: u8| {
                if alpha == 0 {
                    0
                } else {
                    ((value as u32 * 255) / alpha as u32).min(255) as u8
                }
            };
            target[0] = unmultiply(source[2]);
            target[1] = unmultiply(source[1]);
            target[2] = unmultiply(source[0]);
            target[3] = alpha;
        }

        let image = image::RgbaImage::from_raw(from.cx as u32, from.cy as u32, rgba)?;
        let resized = image::imageops::resize(
            &image,
            to.cx as u32,
            to.cy as u32,
            image::imageops::FilterType::Lanczos3,
        );
        rgba_to_bitmap(resized.as_raw(), to)
    }
}

pub(crate) fn rgba_to_bitmap(rgba: &[u8], size: SIZE) -> Option<HBITMAP> {
    unsafe {
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.cx,
                biHeight: -size.cy,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(None, &header, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), rgba.len());
        for (source, target) in rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(pixels.as_chunks_mut::<4>().0)
        {
            let alpha = u16::from(source[3]);
            target[0] = (u16::from(source[2]) * alpha / 255) as u8;
            target[1] = (u16::from(source[1]) * alpha / 255) as u8;
            target[2] = (u16::from(source[0]) * alpha / 255) as u8;
            target[3] = source[3];
        }
        Some(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実体に触れられないパスでも型アイコンは返す。返せないと候補行の
    /// アイコンが丸ごと空になる (`draw_path_icon` にフォールバックが無い)。
    #[test]
    fn falls_back_to_type_icon_for_unreachable_paths() {
        for path in [
            r"C:\__waypoint_no_such_dir__",
            r"C:\__waypoint_no_such_file__.txt",
            r"Z:\",
            r"\\no-such-host-waypoint\share\file.txt",
        ] {
            let icon = system_icon(path, 26);
            assert!(icon.is_some(), "{path} のアイコンが取れていない");
            unsafe {
                let _ = DestroyIcon(icon.unwrap());
            }
        }
    }

    /// アルファを持たないアイコンでも、マスクで抜いた部分以外は不透明になる。
    #[test]
    fn restores_alpha_from_black_and_white_backgrounds() {
        // 1 画素目: 不透明な赤 (BGR = 0,0,200)。2 画素目: 透過部分
        let on_black = [0, 0, 200, 0, 0, 0, 0, 0];
        let mut on_white = [0, 0, 200, 0, 255, 255, 255, 0];
        alpha_from_backgrounds(&on_black, &mut on_white);
        assert_eq!(on_white, [0, 0, 200, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn detects_whether_any_pixel_has_alpha() {
        assert!(!has_alpha(&[10, 20, 30, 0, 40, 50, 60, 0]));
        assert!(has_alpha(&[10, 20, 30, 0, 40, 50, 60, 1]));
    }
}
