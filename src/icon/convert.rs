//! HICON/HBITMAP 変換・リサイズ系のヘルパー。

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetObjectW,
    HBITMAP, HGDIOBJ, SelectObject,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON, SHGFI_SYSICONINDEX,
    SHGetFileInfoW, SHGetImageList, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
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
pub(crate) fn system_icon(path: &str, wanted: i32) -> Option<HICON> {
    unsafe {
        let wide = HSTRING::from(path);
        let mut info = SHFILEINFOW::default();

        // まずシステムイメージリストから引く。こちらのほうが
        // ハンドルを増やさずに済む
        let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON;
        let ok = SHGetFileInfoW(
            &wide,
            Default::default(),
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
            Default::default(),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | icon_size_flag(wanted),
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
        let drawn = windows::Win32::UI::WindowsAndMessaging::DrawIconEx(
            hdc,
            0,
            0,
            icon,
            size.cx,
            size.cy,
            0,
            None,
            windows::Win32::UI::WindowsAndMessaging::DI_NORMAL,
        );
        SelectObject(hdc, old);
        let _ = DeleteDC(hdc);

        drawn.is_ok().then_some(bitmap)
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
