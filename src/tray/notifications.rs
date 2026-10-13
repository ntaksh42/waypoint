//! 自動最適化の保存結果をトレイから知らせる。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_INFO, NIIF_INFO, NIIF_WARNING, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};

pub(super) fn show_optimization_result(hwnd: HWND, saved: bool, message: &str) {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: super::TRAY_UID,
        uFlags: NIF_INFO,
        dwInfoFlags: if saved { NIIF_INFO } else { NIIF_WARNING },
        ..Default::default()
    };
    super::write_tip(&mut data.szInfoTitle, "Azure DevOps optimization");
    super::write_tip(&mut data.szInfo, message);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}
