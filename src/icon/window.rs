//! Current Windows 項目のウィンドウアイコン取得。

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::WindowsAndMessaging::{
    GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, GetWindowThreadProcessId, HICON, ICON_BIG,
    ICON_SMALL, ICON_SMALL2, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};

use super::cached_bitmap_async;
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
    let raw = hwnd.0 as isize;
    cached_bitmap_async(&key, move || unsafe {
        let hwnd = HWND(raw as *mut _);
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
        for which in order {
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
            // 応答が無い (タイムアウト・ハング) ウィンドウは、残りの種別を
            // 試しても同じだけ待たされる。1 回目で諦めて呼び出し側のクラス
            // アイコンへ回す
            if sent.0 == 0 {
                return None;
            }
            if result != 0 {
                return Some(HICON(result as *mut _));
            }
        }
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};
    use windows::core::w;

    #[test]
    fn unresponsive_window_icon_does_not_stall_drawing() {
        let (created, window) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            let hwnd = unsafe {
                CreateWindowExW(
                    Default::default(),
                    w!("STATIC"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            created.send(hwnd.0 as isize).unwrap();
            // メッセージを処理しないウィンドウで旧経路の待ちを再現する。
            let _ = finished.recv();
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        });
        let hwnd = HWND(window.recv_timeout(Duration::from_secs(5)).unwrap() as *mut _);
        let started = Instant::now();
        assert!(unsafe { window_icon_via_message(hwnd, true) }.is_none());
        let blocking = started.elapsed();
        let started = Instant::now();
        assert!(bitmap_for_window_sized(hwnd, 32).is_none());
        let drawing = started.elapsed();
        println!("unresponsive window: synchronous={blocking:?}, drawing={drawing:?}");
        // スケジューラの揺れを許容しても、外部応答の待ちが表示へ乗らない。
        // 1 回目のタイムアウト (100ms) で諦める。以前は 3 回試して約 300ms だった
        assert!(blocking >= Duration::from_millis(80));
        assert!(blocking < Duration::from_millis(250));
        assert!(drawing < blocking / 2);
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        let key = format!("window-icon:32:{}:{pid}", hwnd.0 as isize);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            super::super::apply_ready();
            if super::super::CACHE.with(|cache| cache.borrow().contains_key(&key)) {
                break;
            }
            assert!(Instant::now() < deadline, "icon result was not applied");
            std::thread::sleep(Duration::from_millis(10));
        }
        finish.send(()).unwrap();
        owner.join().unwrap();
        super::super::clear_cache();
    }
}
