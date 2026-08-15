//! メニュー項目のアイコン取得 (FR-2.3) 。
//!
//! `SHGetFileInfo` が返す HICON をビットマップに変換して
//! `SetMenuItemInfo` に渡す。取得はパス単位でキャッシュする
//! (同じフォルダを何度も引かない)。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, HBITMAP, HGDIOBJ, SelectObject,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON,
    SHGFI_SYSICONINDEX, SHGSI_ICON, SHGSI_LARGEICON, SHGSI_SMALLICON, SHGetFileInfoW,
    SHGetImageList, SHGetStockIconInfo, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL,
    SHParseDisplayName, SHSTOCKICONID, SHSTOCKICONINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, HICON, ICON_BIG, ICON_SMALL,
    ICON_SMALL2, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};
use windows::core::{HSTRING, PCWSTR};

thread_local! {
    /// パス -> ビットマップ。メニュー再構築のたびに引き直さない。
    static CACHE: RefCell<HashMap<String, isize>> = RefCell::new(HashMap::new());
    /// 設定 (`settings.menu.iconSize`) の論理サイズ。0 ならシステム既定。
    static ICON_SIZE: Cell<u32> = const { Cell::new(0) };
}

/// メニューアイコンの論理サイズを設定から取り込む。
///
/// 寸法が変わると既存のビットマップは使えないので、
/// 実際に変わったときだけキャッシュを捨てる。
pub fn set_icon_size(size: u32) {
    let changed = ICON_SIZE.with(|current| current.replace(size) != size);
    if changed {
        clear_cache();
    }
}

/// メニューに描くアイコンの寸法。オーナードローの採寸に使う。
pub fn icon_extent() -> SIZE {
    menu_icon_size()
}

/// 描画寸法に見合うシステムイメージリストを選ぶ。
///
/// 16px のアイコンを 32px へ引き伸ばすと輪郭がにじむ。要求寸法以上で
/// 最も近いリストから取れば、拡大ではなく縮小になり輪郭が保たれる。
fn image_list_for(size: i32) -> i32 {
    let list = if size <= 16 {
        SHIL_SMALL
    } else if size <= 32 {
        SHIL_LARGE
    } else if size <= 48 {
        SHIL_EXTRALARGE
    } else {
        SHIL_JUMBO
    };
    list as i32
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
    bitmap_for_sized(path, menu_icon_size().cx)
}

/// 指定パスのアイコンを、メニューの iconSize 設定とは独立に
/// 指定寸法のビットマップとして得る。
///
/// Quick Launch のようにメニューと別の寸法で描くと、`bitmap_for` が
/// 返すビットマップとの寸法差で AlphaBlend が拡大縮小を行いにじむ
/// (要求寸法どおりのビットマップを直接取れば等倍コピーで済む)。
pub fn bitmap_for_sized(path: &str, size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("{size}:{path}"), || load_bitmap(path, size))
}

/// Windows 標準の操作アイコンをメニュー用ビットマップとして得る。
pub fn bitmap_for_stock(id: SHSTOCKICONID) -> Option<HBITMAP> {
    bitmap_for_stock_sized(id, menu_icon_size().cx)
}

/// 指定 ID の標準アイコンを、メニューの iconSize 設定とは独立に
/// 指定寸法のビットマップとして得る (`bitmap_for_sized` と同じ理由)。
pub fn bitmap_for_stock_sized(id: SHSTOCKICONID, size: i32) -> Option<HBITMAP> {
    let key = format!("stock-icon:{size}:{}", id.0);
    cached_bitmap(&key, || unsafe {
        let mut info = SHSTOCKICONINFO {
            cbSize: size_of::<SHSTOCKICONINFO>() as u32,
            ..Default::default()
        };
        // 32px 描画で小アイコンを引き伸ばすとにじむので寸法に合わせて要求する
        let size_flag = if size <= 16 {
            SHGSI_SMALLICON
        } else {
            SHGSI_LARGEICON
        };
        SHGetStockIconInfo(id, SHGSI_ICON | size_flag, &mut info).ok()?;
        let bitmap = icon_to_bitmap(info.hIcon, SIZE { cx: size, cy: size });
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
        let mut large = HICON::default();
        let mut small = HICON::default();
        // 大小の両方を取り、描画寸法に近いほうを使う。16px を引き伸ばすとにじむ
        let extracted = ExtractIconExW(&path, index, Some(&mut large), Some(&mut small), 1);
        if extracted == 0 {
            return None;
        }
        let prefer_large = menu_icon_size().cx > 16 && !large.is_invalid();
        let chosen = if prefer_large { large } else { small };
        let bitmap = (!chosen.is_invalid())
            .then(|| icon_to_bitmap(chosen, menu_icon_size()))
            .flatten();
        if !large.is_invalid() {
            let _ = DestroyIcon(large);
        }
        if !small.is_invalid() {
            let _ = DestroyIcon(small);
        }
        bitmap
    })
}

/// Current Windows の各項目にそのウィンドウのアイコンを付ける。
///
/// Explorer (`CabinetWClass`) などはクラスアイコンを登録せず、
/// `WM_GETICON` に応答する形でアイコンを渡す (実測でクラス側は
/// large=0/small=0 だった) 。`WM_GETICON` を先に試し、無応答なら
/// クラスアイコンへフォールバックする。
pub fn bitmap_for_window(hwnd: HWND) -> Option<HBITMAP> {
    bitmap_for_window_sized(hwnd, menu_icon_size().cx)
}

/// `bitmap_for_window` のメニュー設定に依存しない版。[`bitmap_for_sized`] 参照。
pub fn bitmap_for_window_sized(hwnd: HWND, size: i32) -> Option<HBITMAP> {
    let key = format!("window-icon:{size}:{}", hwnd.0 as isize);
    cached_bitmap(&key, || unsafe {
        let large_first = size > 16;
        let icon = window_icon_via_message(hwnd, large_first)
            .or_else(|| window_icon_via_class(hwnd, large_first))?;
        icon_to_bitmap(icon, SIZE { cx: size, cy: size })
    })
}

/// `WM_GETICON` でウィンドウ自身が渡すアイコンを取る。
///
/// 応答しないウィンドウでハングしないよう `SendMessageTimeoutW` を使う。
unsafe fn window_icon_via_message(hwnd: HWND, large_first: bool) -> Option<HICON> {
    let order: [WPARAM; 3] = if large_first {
        [
            WPARAM(ICON_BIG as usize),
            WPARAM(ICON_SMALL2 as usize),
            WPARAM(ICON_SMALL as usize),
        ]
    } else {
        [
            WPARAM(ICON_SMALL as usize),
            WPARAM(ICON_SMALL2 as usize),
            WPARAM(ICON_BIG as usize),
        ]
    };
    unsafe {
        order.into_iter().find_map(|which| {
            let mut result = 0usize;
            let sent = SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                which,
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                Some(&mut result),
            );
            (sent.0 != 0 && result != 0).then_some(HICON(result as *mut _))
        })
    }
}

/// クラスアイコン (`GCLP_HICON` / `GCLP_HICONSM`) へのフォールバック。
unsafe fn window_icon_via_class(hwnd: HWND, large_first: bool) -> Option<HICON> {
    let (first, second) = if large_first {
        (GCLP_HICON, GCLP_HICONSM)
    } else {
        (GCLP_HICONSM, GCLP_HICON)
    };
    unsafe {
        let raw = GetClassLongPtrW(hwnd, first);
        let raw = if raw == 0 {
            GetClassLongPtrW(hwnd, second)
        } else {
            raw
        };
        (raw != 0).then_some(HICON(raw as *mut _))
    }
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
        let bitmap = icon_to_bitmap(icon, menu_icon_size());
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

fn load_bitmap(path: &str, size: i32) -> Option<HBITMAP> {
    let icon = system_icon(path, size)?;
    let bmp = icon_to_bitmap(icon, SIZE { cx: size, cy: size });
    unsafe {
        let _ = DestroyIcon(icon);
    }
    bmp
}

/// シェルが返すアイコンを、描画寸法に見合う解像度で取得する。
fn system_icon(path: &str, wanted: i32) -> Option<HICON> {
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
            SHGetImageList::<IImageList>(image_list_for(menu_icon_size().cx))
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
fn icon_to_bitmap(icon: HICON, size: SIZE) -> Option<HBITMAP> {
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
///
/// 設定値 (`settings.menu.iconSize`) は 96dpi 基準の論理サイズ。
/// `SM_CXSMICON` は DPI 適用済みの値が返るので、これを基準寸法で
/// 割った比を倍率として使い、設定値へ同じ拡大を掛ける。
fn menu_icon_size() -> SIZE {
    let system = system_small_icon_size();
    let configured = ICON_SIZE.with(Cell::get);
    if configured == 0 {
        return system;
    }
    let scaled = scale_icon_size(configured, system.cx);
    SIZE {
        cx: scaled,
        cy: scaled,
    }
}

/// システムが定める小アイコンの寸法。DPI 適用済みの値が返る。
fn system_small_icon_size() -> SIZE {
    unsafe {
        let cx = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
        );
        let cy = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSMICON,
        );
        SIZE {
            cx: if cx > 0 { cx } else { BASE_ICON_SIZE },
            cy: if cy > 0 { cy } else { BASE_ICON_SIZE },
        }
    }
}

/// 96dpi でのメニューアイコン寸法。DPI 倍率はこれを基準に求める。
const BASE_ICON_SIZE: i32 = 16;

/// 論理サイズへ DPI 倍率を掛ける。倍率は `system / 16`。
///
/// 100% 表示なら設定値がそのまま出る。極端な値でメニューが
/// 壊れないよう上下を留める。
fn scale_icon_size(configured: u32, system_cx: i32) -> i32 {
    let configured = configured.clamp(16, 64) as i32;
    let scaled = configured * system_cx.max(BASE_ICON_SIZE) / BASE_ICON_SIZE;
    scaled.clamp(BASE_ICON_SIZE, 256)
}

#[cfg(test)]
mod tests {
    use super::{BASE_ICON_SIZE, scale_icon_size};

    /// 100% 表示 (SM_CXSMICON が 16) では設定値がそのまま出る。
    #[test]
    fn uses_the_configured_size_at_100_percent() {
        assert_eq!(scale_icon_size(32, BASE_ICON_SIZE), 32);
        assert_eq!(scale_icon_size(16, BASE_ICON_SIZE), 16);
    }

    /// 150% 表示 (SM_CXSMICON が 24) では設定値も 1.5 倍になる。
    #[test]
    fn scales_with_the_system_metric() {
        assert_eq!(scale_icon_size(32, 24), 48);
    }

    /// 設定値が範囲外でもメニューが壊れる寸法にはしない。
    #[test]
    fn keeps_out_of_range_values_usable() {
        assert_eq!(scale_icon_size(0, BASE_ICON_SIZE), 16);
        assert_eq!(scale_icon_size(4096, BASE_ICON_SIZE), 64);
    }
}
