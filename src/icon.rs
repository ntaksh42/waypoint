//! メニュー項目のアイコン取得 (FR-2.3) 。
//!
//! `SHGetFileInfo` が返す HICON をビットマップに変換して
//! `SetMenuItemInfo` に渡す。取得はパス単位でキャッシュする
//! (同じフォルダを何度も引かない)。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, HBITMAP, HGDIOBJ, SelectObject,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGetFileInfoW, SHGetImageList,
    SHIL_SMALL,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::core::HSTRING;

thread_local! {
    /// パス -> ビットマップ。メニュー再構築のたびに引き直さない。
    static CACHE: RefCell<HashMap<String, isize>> = RefCell::new(HashMap::new());
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
