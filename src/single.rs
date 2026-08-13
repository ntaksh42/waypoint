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
///
/// `GetLastError` は `CreateMutexW` の**直後**に読むこと。
/// Result を分解するだけでも別の Win32 呼び出しが挟まり、
/// ERROR_ALREADY_EXISTS(183) が別の値に上書きされる。
pub fn acquire() -> Result<InstanceGuard, AlreadyRunning> {
    unsafe {
        // 所有権は取らない (第2引数 false) 。
        // 存在するかどうかだけが判定材料で、所有権を握ると
        // 強制終了されたプロセスの mutex が abandoned 状態で残り、
        // 次の起動が待たされる。
        let result = CreateMutexW(None, false, w!(r"Local\waypoint_single_instance"));
        // ここを 1 行でも後ろにずらすと壊れる
        let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;

        match result {
            Ok(handle) => {
                if already_exists {
                    Err(AlreadyRunning)
                } else {
                    Ok(InstanceGuard(handle))
                }
            }
            // Mutex を作れない場合は起動を止めない
            Err(_) => Ok(InstanceGuard(HANDLE::default())),
        }
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
