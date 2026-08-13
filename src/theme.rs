//! ネイティブのポップアップメニューをダーク表示にする。

use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

/// Windows 10 1903 以降の uxtheme が持つメニューテーマを有効にする。
///
/// 公開 API ではポップアップメニューのダーク表示を指定できないため、
/// uxtheme の ordinal を実行時に解決する。解決できない環境では何もしない。
pub fn enable_dark_menus() {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();

    unsafe {
        let Ok(module) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        let Some(set_mode) = GetProcAddress(module, PCSTR(135usize as *const u8)) else {
            return;
        };
        let Some(flush) = GetProcAddress(module, PCSTR(136usize as *const u8)) else {
            return;
        };

        let set_mode: SetPreferredAppMode = std::mem::transmute(set_mode);
        let flush: FlushMenuThemes = std::mem::transmute(flush);
        const FORCE_DARK: i32 = 2;
        set_mode(FORCE_DARK);
        flush();
        // uxtheme はプロセス終了までメニューテーマを提供するため解放しない。
    }
}
