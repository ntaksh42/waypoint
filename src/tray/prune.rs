//! 存在しなくなった登録項目の定期削除 (FR-7.8)。
//!
//! 存在確認は別スレッドで行い、結果を `WM_MISSING_ITEMS_FOUND` で受け取って
//! から UI スレッドで config へ適用する。確認中に設定画面から保存された
//! 場合も、その時点の config に対して消すので編集内容を巻き戻さない。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SetTimer, WM_APP};

use super::with_state;
use crate::quick_launch_window;

pub(crate) const WM_MISSING_ITEMS_FOUND: u32 = WM_APP + 11;
pub(crate) const PRUNE_TIMER_ID: usize = 2;
const PRUNE_INTERVAL_MS: u32 = 10 * 60 * 1000;

static SCANNING: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 起動直後に 1 回走らせ、以後 10 分おきに走らせる。
pub fn start_prune_timer(hwnd: HWND) {
    scan_async(hwnd);
    unsafe {
        let _ = SetTimer(Some(hwnd), PRUNE_TIMER_ID, PRUNE_INTERVAL_MS, None);
    }
}

pub(crate) fn scan_async(hwnd: HWND) {
    let Some(paths) = with_state(|s| s.borrow().as_ref().map(|st| st.config.item_paths())) else {
        return;
    };
    if paths.is_empty() || SCANNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let notify = hwnd.0 as isize;
    std::thread::spawn(move || {
        let missing = crate::config::find_missing(&paths, |p| p.exists());
        let found = !missing.is_empty();
        *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = missing;
        SCANNING.store(false, Ordering::Release);
        if found {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(notify as *mut _)),
                    WM_MISSING_ITEMS_FOUND,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    });
}

/// `WM_MISSING_ITEMS_FOUND` を受けて config から取り除き、保存して
/// Quick Launch の候補を組み直す。
pub(crate) fn apply_missing() {
    let missing = std::mem::take(&mut *RESULT.lock().unwrap_or_else(|e| e.into_inner()));
    with_state(|s| {
        let mut state = s.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let removed = state.config.remove_paths(&missing);
        if removed.is_empty() {
            return;
        }
        if let Err(e) = crate::config::save(&state.config) {
            crate::panic_log::record(&format!("failed to save after removing missing items: {e}"));
            return;
        }
        for (name, path) in &removed {
            crate::panic_log::record(&format!("removed missing item \"{name}\": {path}"));
        }
        quick_launch_window::configure_config_items(&state.config, &state.dynamic);
    });
}
