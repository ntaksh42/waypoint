//! クリップボードへのテキストコピー (Quick Launch の `Ctrl+C`)。

use windows::Win32::Foundation::{GlobalFree, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// クリップボードへ UTF-16 テキストとして書き込む。
///
/// `GlobalAlloc` で確保したメモリの所有権は `SetClipboardData` 成功後は
/// システムへ渡る (呼び出し側で解放しない)。失敗時のみ自前で解放する。
pub fn set_text(text: &str) -> windows::core::Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len() * size_of::<u16>();

    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            let _ = GlobalFree(Some(handle));
            return Err(windows::core::Error::empty());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.cast(), wide.len());
        let _ = GlobalUnlock(handle);

        if let Err(e) = OpenClipboard(None) {
            let _ = GlobalFree(Some(handle));
            return Err(e);
        }
        if let Err(e) = EmptyClipboard() {
            let _ = CloseClipboard();
            let _ = GlobalFree(Some(handle));
            return Err(e);
        }
        if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))) {
            let _ = CloseClipboard();
            let _ = GlobalFree(Some(handle));
            return Err(e);
        }
        let _ = CloseClipboard();
    }
    Ok(())
}
