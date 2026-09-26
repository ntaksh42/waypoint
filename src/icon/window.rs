//! Current Windows 項目のウィンドウアイコン取得。

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::WindowsAndMessaging::{
    GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, GetWindowThreadProcessId, HICON, ICON_BIG,
    ICON_SMALL, ICON_SMALL2, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};

use super::cached_bitmap;
use super::convert::{icon_to_bitmap, load_bitmap};

/// Current Windows の各項目にそのウィンドウのアイコンを付ける。
///
/// Explorer (`CabinetWClass`) などはクラスアイコンを登録せず、
/// `WM_GETICON` に応答する形でアイコンを渡す (実測でクラス側は
/// large=0/small=0 だった) 。`WM_GETICON` を先に試し、無応答なら
/// クラスアイコンへフォールバックする。
/// 指定ウィンドウのアイコンを指定寸法でビットマップ化する。
///
/// `HWND` は閉じたウィンドウの分を OS が再利用するため、そのままキーに
/// すると別のウィンドウが前の結果を引き継ぐ。所有プロセスを混ぜて分ける。
pub(crate) fn bitmap_for_window_sized(hwnd: HWND, size: i32) -> Option<HBITMAP> {
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    let key = format!("window-icon:{size}:{}:{process_id}", hwnd.0 as isize);
    cached_bitmap(&key, || unsafe {
        let large_first = size > 16;
        window_icon_via_message(hwnd, large_first)
            .or_else(|| window_icon_via_class(hwnd, large_first))
            .and_then(|icon| icon_to_bitmap(icon, SIZE { cx: size, cy: size }))
            .or_else(|| window_icon_via_executable(hwnd, size))
    })
}

/// ウィンドウ自身もクラスもアイコンを持たないときは、所有プロセスの
/// 実行ファイルのアイコンで代用する。
///
/// `WM_GETICON` にもクラスアイコンにも何も無いウィンドウは珍しくない
/// (UWP の `ApplicationFrameWindow`、アイコンを登録しないツール類、
/// 応答が 100ms を超えたウィンドウ)。代用しないと候補行のアイコンが空になる。
fn window_icon_via_executable(hwnd: HWND, size: i32) -> Option<HBITMAP> {
    let path = crate::process::process_path_of(hwnd)?;
    load_bitmap(&path, size)
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
