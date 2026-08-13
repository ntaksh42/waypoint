//! 二重起動の抑止 (FR-8.3) 。
//!
//! 名前付き Mutex で先行プロセスを検出し、二つ目は既存プロセスに
//! メニュー表示を要求して終了する。

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::w;

use crate::trigger::WM_TRIGGER_MENU;

/// Mutex を保持するだけの型。プロセス終了まで生かしておく。
pub struct InstanceGuard(#[allow(dead_code)] HANDLE);

/// 先行プロセスが既に動いている。
#[derive(Debug)]
pub struct AlreadyRunning;

/// 先行プロセスがあれば Err を返す。
pub fn acquire() -> Result<InstanceGuard, AlreadyRunning> {
    unsafe {
        let Ok(handle) = CreateMutexW(None, true, w!("Global\\waypoint_single_instance")) else {
            // Mutex を作れない場合は起動を止めない
            return Ok(InstanceGuard(HANDLE::default()));
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Err(AlreadyRunning);
        }
        Ok(InstanceGuard(handle))
    }
}

/// 既に動いているプロセスにメニュー表示を要求する。
///
/// メッセージ専用ウィンドウは FindWindowW では見つからないため、
/// 通常のウィンドウとして作った受け口をクラス名で探す。
pub fn signal_existing() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(w!("WaypointMessageWindow"), None) {
            let _ = PostMessageW(
                Some(hwnd),
                WM_TRIGGER_MENU,
                Default::default(),
                Default::default(),
            );
        }
    }
}
