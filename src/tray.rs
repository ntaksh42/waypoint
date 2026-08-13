//! タスクトレイ常駐とその右クリックメニュー。
//!
//! ウィンドウは表示しないが、Shell_NotifyIcon と RegisterHotKey が
//! ウィンドウメッセージを必要とするため、メッセージ専用ウィンドウ
//! (HWND_MESSAGE の子) を 1 つだけ作って受け口にする。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, HMENU, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MF_SEPARATOR, MF_STRING,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY,
    WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{PCWSTR, Result, w};

/// トレイアイコンからの通知を受け取るための独自メッセージ。
/// WM_APP 以降はアプリが自由に使える領域。
const WM_TRAY: u32 = WM_APP + 1;

/// トレイアイコンの識別子。1 プロセス 1 アイコンなので固定値でよい。
const TRAY_UID: u32 = 1;

// メニュー項目 ID
const ID_SETTINGS: usize = 1;
const ID_RELOAD: usize = 2;
const ID_EXIT: usize = 3;

thread_local! {
    /// メニュー表示中に再入するのを防ぐ。TrackPopupMenuEx は
    /// モーダルループを回すため、その間に届いた通知で二重に開かないようにする。
    static MENU_OPEN: Cell<bool> = const { Cell::new(false) };
}

/// トレイに常駐する。戻り値はメッセージ専用ウィンドウのハンドル。
pub fn install() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;

        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("WaypointMessageWindow"),
            ..Default::default()
        };
        // 失敗しても続行できないので戻り値を確認する
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        // HWND_MESSAGE の子はタスクバーにもデスクトップにも現れない
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("WaypointMessageWindow"),
            w!("waypoint"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )?;

        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            ..Default::default()
        };
        write_tip(&mut data.szTip, "waypoint");

        Shell_NotifyIconW(NIM_ADD, &data).ok()?;

        Ok(hwnd)
    }
}

/// トレイアイコンを取り除く。終了時に呼ばないとアイコンが残る。
pub fn remove(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

/// szTip は固定長の UTF-16 配列。末尾の NUL を残して切り詰める。
fn write_tip(dst: &mut [u16; 128], text: &str) {
    let src: Vec<u16> = text.encode_utf16().collect();
    let len = src.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&src[..len]);
    dst[len] = 0;
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            // 下位ワードにマウスメッセージが入る
            let event = (lparam.0 as u32) & 0xffff;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_context_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn show_context_menu(hwnd: HWND) {
    // TrackPopupMenuEx はモーダル。再入すると入れ子のループになる
    if MENU_OPEN.with(|f| f.replace(true)) {
        return;
    }

    let selected = unsafe { track_menu(hwnd) };

    MENU_OPEN.with(|f| f.set(false));

    match selected {
        ID_SETTINGS => { /* TODO: 管理画面 (FR-6) */ }
        ID_RELOAD => { /* TODO: 設定再読み込み (FR-8.2) */ }
        ID_EXIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

/// メニューを構築して表示し、選ばれた項目 ID を返す。0 は取り消し。
unsafe fn track_menu(hwnd: HWND) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };

        let built = build_items(menu);
        let mut pos = POINT::default();
        let got_pos = GetCursorPos(&mut pos).is_ok();

        let selected = if built.is_ok() && got_pos {
            // これを呼ばないとメニュー外をクリックしても閉じない
            let _ = SetForegroundWindow(hwnd);
            TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
                pos.x,
                pos.y,
                hwnd,
                None,
            )
            .0 as usize
        } else {
            0
        };

        let _ = DestroyMenu(menu);
        selected
    }
}

unsafe fn build_items(menu: HMENU) -> Result<()> {
    unsafe {
        AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings"))?;
        AppendMenuW(menu, MF_STRING, ID_RELOAD, w!("Reload config"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"))?;
        Ok(())
    }
}
