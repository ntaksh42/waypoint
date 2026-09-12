//! Quick Launch とトレイのアイコン取得。

mod convert;
mod scale;
mod window;

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::core::HSTRING;

use convert::{icon_to_bitmap, load_bitmap, rgba_to_bitmap, shell_icon};
pub(crate) use window::bitmap_for_window_sized;

thread_local! {
    /// パス -> ビットマップ。Quick Launch の再描画ごとに引き直さない。
    static CACHE: RefCell<HashMap<String, isize>> = RefCell::new(HashMap::new());
}

/// 設定 (歯車) アイコンの在り処。
///
/// `SIID_SETTINGS` は実測で中身が空だったため使えない。
/// shell32.dll の 314 番が単体の歯車で、16px でも形が潰れない。
const SHELL32: &str = "shell32.dll";
const GEAR_INDEX: i32 = 314;

/// トレイの「設定」項目に使う歯車アイコン。
pub(crate) fn bitmap_for_settings_sized(size: i32) -> Option<HBITMAP> {
    bitmap_for_dll_icon(SHELL32, GEAR_INDEX, size)
}

/// 指定パスのアイコンを、メニューの iconSize 設定とは独立に
/// 指定寸法のビットマップとして得る。
///
/// Quick Launch のようにメニューと別の寸法で描くと、`bitmap_for` が
/// 返すビットマップとの寸法差で AlphaBlend が拡大縮小を行いにじむ
/// (要求寸法どおりのビットマップを直接取れば等倍コピーで済む)。
pub(crate) fn bitmap_for_sized(path: &str, size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("{size}:{path}"), || load_bitmap(path, size))
}

/// DLL に埋め込まれたアイコンをインデックス指定で取得する。
///
/// `SHGetStockIconInfo` は ID によっては中身が空のアイコンを返す
/// (実測: `SIID_SETTINGS` は全ピクセルが透明で、メニューには何も
/// 表示されない)。歯車のように標準 ID から取れないものは、
/// シェルの DLL から直接引く。
fn bitmap_for_dll_icon(dll: &str, index: i32, size: i32) -> Option<HBITMAP> {
    let key = format!("dll-icon:{size}:{dll}:{index}");
    cached_bitmap(&key, || unsafe {
        let path = HSTRING::from(dll);
        let mut large = HICON::default();
        let mut small = HICON::default();
        // 大小の両方を取り、描画寸法に近いほうを使う。
        let extracted = ExtractIconExW(&path, index, Some(&mut large), Some(&mut small), 1);
        if extracted == 0 {
            return None;
        }
        let prefer_large = size > 16 && !large.is_invalid();
        let chosen = if prefer_large { large } else { small };
        let bitmap = (!chosen.is_invalid())
            .then(|| icon_to_bitmap(chosen, SIZE { cx: size, cy: size }))
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

/// 埋め込み PNG を指定寸法へ縮小して使う。
pub(crate) fn bitmap_for_asset_sized(key: &str, png: &[u8], size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("asset-icon:{size}:{key}"), || {
        let target = SIZE { cx: size, cy: size };
        let image = image::load_from_memory(png).ok()?.into_rgba8();
        let image = image::imageops::resize(
            &image,
            target.cx as u32,
            target.cy as u32,
            image::imageops::FilterType::Lanczos3,
        );
        rgba_to_bitmap(image.as_raw(), target)
    })
}

/// ブックマーク URL に対応する favicon を、Chrome/Edge の `Favicons` DB
/// から得て Quick Launch 用ビットマップにする。見つからなければ None
/// (呼び出し側が汎用のリンクアイコンへフォールバックする)。
pub(crate) fn bitmap_for_favicon_sized(url: &str, size: i32) -> Option<HBITMAP> {
    cached_bitmap(&format!("favicon:{size}:{url}"), || {
        let png = crate::favicons::lookup(url)?;
        let image = image::load_from_memory(&png).ok()?.into_rgba8();
        let target = SIZE { cx: size, cy: size };
        let image = if image.width() as i32 == size && image.height() as i32 == size {
            image
        } else {
            image::imageops::resize(
                &image,
                target.cx as u32,
                target.cy as u32,
                image::imageops::FilterType::Lanczos3,
            )
        };
        rgba_to_bitmap(image.as_raw(), target)
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
pub(crate) fn bitmap_for_shell_sized(target: &str, size: i32) -> Option<HBITMAP> {
    let key = format!("shell-namespace:{size}:{target}");
    let cached = CACHE.with(|cache| cache.borrow().get(&key).copied());
    if let Some(raw) = cached {
        return (raw != 0).then_some(HBITMAP(raw as *mut _));
    }

    let bitmap = shell_icon(target, size).and_then(|icon| {
        let bitmap = icon_to_bitmap(icon, SIZE { cx: size, cy: size });
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
///
/// `HashMap` を空にするだけでは中身の `HBITMAP` は解放されない。
/// テーマ変更・設定再読み込みのたびに全件 GDI リークし、頻繁な
/// 切り替えでプロセスの GDI ハンドル上限に達してアイコンが描けなく
/// なる (実測で確認済み) 。値を読んでから `DeleteObject` する。
pub(crate) fn clear_cache() {
    CACHE.with(|c| {
        for (_, raw) in c.borrow_mut().drain() {
            if raw != 0 {
                unsafe {
                    let _ = DeleteObject(HBITMAP(raw as *mut _).into());
                }
            }
        }
    });
}
