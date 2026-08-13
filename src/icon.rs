//! メニュー項目のアイコン取得 (FR-2.3) 。
//!
//! `SHGetFileInfo` が返す HICON をビットマップに変換して
//! `SetMenuItemInfo` に渡す。取得はパス単位でキャッシュする
//! (同じフォルダを何度も引かない)。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::{HWND, SIZE};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, HBITMAP, HGDIOBJ, SelectObject,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHFILEINFOW, SHGFI_ICON, SHGFI_PIDL, SHGFI_SMALLICON, SHGFI_SYSICONINDEX,
    SHGSI_ICON, SHGSI_SMALLICON, SHGetFileInfoW, SHGetImageList, SHGetStockIconInfo, SHIL_SMALL,
    SHParseDisplayName, SHSTOCKICONID, SHSTOCKICONINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, HICON,
};
use windows::core::{HSTRING, PCWSTR};

thread_local! {
    /// パス -> ビットマップ。メニュー再構築のたびに引き直さない。
    static CACHE: RefCell<HashMap<String, isize>> = RefCell::new(HashMap::new());
}

/// 設定 (歯車) アイコンの在り処。
///
/// `SIID_SETTINGS` は実測で中身が空だったため使えない。
/// shell32.dll の 314 番が単体の歯車で、16px でも形が潰れない。
const SHELL32: &str = "shell32.dll";
const GEAR_INDEX: i32 = 314;

/// メニューの「設定」項目に使う歯車アイコン。
pub fn bitmap_for_settings() -> Option<HBITMAP> {
    bitmap_for_dll_icon(SHELL32, GEAR_INDEX)
}

/// 指定パスのアイコンをメニュー用ビットマップとして得る。
///
/// 失敗しても致命的ではないので None を返し、アイコンなしで描く。
pub fn bitmap_for(path: &str) -> Option<HBITMAP> {
    let cached = CACHE.with(|c| c.borrow().get(path).copied());
    if let Some(raw) = cached {
        return (raw != 0).then_some(HBITMAP(raw as *mut _));
    }

    let bmp = load_bitmap(path);
    CACHE.with(|c| {
        c.borrow_mut()
            .insert(path.to_string(), bmp.map_or(0, |b| b.0 as isize))
    });
    bmp
}

/// Windows 標準の操作アイコンをメニュー用ビットマップとして得る。
pub fn bitmap_for_stock(id: SHSTOCKICONID) -> Option<HBITMAP> {
    let key = format!("stock-icon:{}", id.0);
    cached_bitmap(&key, || unsafe {
        let mut info = SHSTOCKICONINFO {
            cbSize: size_of::<SHSTOCKICONINFO>() as u32,
            ..Default::default()
        };
        SHGetStockIconInfo(id, SHGSI_ICON | SHGSI_SMALLICON, &mut info).ok()?;
        let bitmap = icon_to_bitmap(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        bitmap
    })
}

/// DLL に埋め込まれたアイコンをインデックス指定で取得する。
///
/// `SHGetStockIconInfo` は ID によっては中身が空のアイコンを返す
/// (実測: `SIID_SETTINGS` は全ピクセルが透明で、メニューには何も
/// 表示されない)。歯車のように標準 ID から取れないものは、
/// シェルの DLL から直接引く。
pub fn bitmap_for_dll_icon(dll: &str, index: i32) -> Option<HBITMAP> {
    let key = format!("dll-icon:{dll}:{index}");
    cached_bitmap(&key, || unsafe {
        let path = HSTRING::from(dll);
        let mut small = HICON::default();
        // 小アイコンのみ要求する。メニューは 16px 相当で描く
        let extracted = ExtractIconExW(&path, index, None, Some(&mut small), 1);
        if extracted == 0 || small.is_invalid() {
            return None;
        }
        let bitmap = icon_to_bitmap(small);
        let _ = DestroyIcon(small);
        bitmap
    })
}

/// Current Windows の各項目に、そのウィンドウクラスのアイコンを付ける。
pub fn bitmap_for_window(hwnd: HWND) -> Option<HBITMAP> {
    let key = format!("window-icon:{}", hwnd.0 as isize);
    cached_bitmap(&key, || unsafe {
        let raw = GetClassLongPtrW(hwnd, GCLP_HICONSM);
        let raw = if raw == 0 {
            GetClassLongPtrW(hwnd, GCLP_HICON)
        } else {
            raw
        };
        (raw != 0)
            .then_some(HICON(raw as *mut _))
            .and_then(icon_to_bitmap)
    })
}

/// 埋め込み PNG を現在の DPI のメニューサイズへ縮小して使う。
pub fn bitmap_for_asset(key: &str, png: &[u8]) -> Option<HBITMAP> {
    cached_bitmap(&format!("asset-icon:{key}"), || {
        let size = menu_icon_size();
        let image = image::load_from_memory(png).ok()?.into_rgba8();
        let image = image::imageops::resize(
            &image,
            size.cx as u32,
            size.cy as u32,
            image::imageops::FilterType::Lanczos3,
        );
        rgba_to_bitmap(image.as_raw(), size)
    })
}

fn cached_bitmap(key: &str, load: impl FnOnce() -> Option<HBITMAP>) -> Option<HBITMAP> {
    let cached = CACHE.with(|cache| cache.borrow().get(key).copied());
    if let Some(raw) = cached {
        return (raw != 0).then_some(HBITMAP(raw as *mut _));
    }
    let bitmap = load();
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(key.to_string(), bitmap.map_or(0, |value| value.0 as isize))
    });
    bitmap
}

/// ファイルパスを持たないシェル名前空間項目のアイコンを得る。
pub fn bitmap_for_shell(target: &str) -> Option<HBITMAP> {
    let key = format!("shell-namespace:{target}");
    let cached = CACHE.with(|cache| cache.borrow().get(&key).copied());
    if let Some(raw) = cached {
        return (raw != 0).then_some(HBITMAP(raw as *mut _));
    }

    let bitmap = shell_icon(target).and_then(|icon| {
        let bitmap = icon_to_bitmap(icon);
        unsafe {
            let _ = DestroyIcon(icon);
        }
        bitmap
    });
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(key, bitmap.map_or(0, |value| value.0 as isize))
    });
    bitmap
}

/// キャッシュを捨てる。テーマ変更や設定再読み込みで呼ぶ。
pub fn clear_cache() {
    CACHE.with(|c| c.borrow_mut().clear());
}

fn load_bitmap(path: &str) -> Option<HBITMAP> {
    let icon = system_icon(path)?;
    let bmp = icon_to_bitmap(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    bmp
}

/// シェルが返す小アイコンを取得する。
fn system_icon(path: &str) -> Option<HICON> {
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
        if ok != 0
            && let Ok(list) = SHGetImageList::<IImageList>(SHIL_SMALL as i32)
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
            SHGFI_ICON | SHGFI_SMALLICON,
        );
        (ok != 0 && !info.hIcon.is_invalid()).then_some(info.hIcon)
    }
}

fn shell_icon(target: &str) -> Option<HICON> {
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
            SHGetImageList::<IImageList>(SHIL_SMALL as i32)
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
fn icon_to_bitmap(icon: HICON) -> Option<HBITMAP> {
    let size = menu_icon_size();
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

fn rgba_to_bitmap(rgba: &[u8], size: SIZE) -> Option<HBITMAP> {
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
        for (source, target) in rgba.chunks_exact(4).zip(pixels.chunks_exact_mut(4)) {
            let alpha = u16::from(source[3]);
            target[0] = (u16::from(source[2]) * alpha / 255) as u8;
            target[1] = (u16::from(source[1]) * alpha / 255) as u8;
            target[2] = (u16::from(source[0]) * alpha / 255) as u8;
            target[3] = source[3];
        }
        Some(bitmap)
    }
}

/// メニューのアイコン寸法。DPI に追従させる。
fn menu_icon_size() -> SIZE {
    unsafe {
        let cx = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
        );
        let cy = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSMICON,
        );
        SIZE {
            cx: if cx > 0 { cx } else { 16 },
            cy: if cy > 0 { cy } else { 16 },
        }
    }
}
