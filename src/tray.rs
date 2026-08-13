//! タスクトレイ常駐と、アプリ全体のメッセージ処理。
//!
//! ウィンドウは見せないが、`HWND_MESSAGE` の子ではなく
//! 「非表示の通常ウィンドウ」にしている。理由は 2 つ:
//!
//! - `TrackPopupMenuEx` は所有ウィンドウを前面化できる必要がある (R-2) 。
//!   メッセージ専用ウィンドウは前面化できない。
//! - 二重起動時に `FindWindowW` で先行プロセスを探す (FR-8.3) 。
//!   メッセージ専用ウィンドウは列挙・検索の対象外になる。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, HMENU, IDI_APPLICATION, LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::config::{Config, LoadOutcome};
use crate::menu::BuiltMenu;
use crate::process;
use crate::shell;
use crate::trigger::{self, WM_TRIGGER_MENU};

/// トレイアイコンからの通知。WM_APP 以降はアプリが自由に使える。
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;

// トレイの右クリックメニューの項目 ID
const ID_SETTINGS: usize = 9001;
const ID_RELOAD: usize = 9002;
const ID_AUTOSTART: usize = 9003;
const ID_EXIT: usize = 9004;

pub const CLASS_NAME: PCWSTR = w!("WaypointMessageWindow");

thread_local! {
    /// 現在の設定と、そこから組み立てたメニュー。
    /// 設定を再読み込みしたら両方入れ替える。
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

struct AppState {
    config: Config,
    menu: Option<BuiltMenu>,
    /// 設定の読み込みに失敗した理由。空メニューで起動した場合に入る。
    load_error: Option<String>,
    /// ホットキーが他アプリに取られていた場合に立つ。
    hotkey_failed: bool,
}

/// 設定を読み込み、メニューを構築して保持する。
pub fn load_state() {
    let (config, load_error) = match crate::config::load() {
        LoadOutcome::Loaded(c) | LoadOutcome::Created(c) => (c, None),
        // 壊れた設定でも起動は続ける (FR-7.4)
        LoadOutcome::Failed(e) => (Config::default(), Some(e)),
    };
    let menu = crate::menu::build(&config).ok();
    STATE.with(|s| {
        *s.borrow_mut() = Some(AppState {
            config,
            menu,
            load_error,
            hotkey_failed: false,
        })
    });
}

/// 設定を読み直してメニューを組み立て直す (FR-8.2) 。
pub fn reload(hwnd: HWND) {
    // ホットキーが変わっているかもしれないので張り直す
    trigger::unregister_hotkey(hwnd);
    load_state();
    let ok = register_hotkey_from_config(hwnd);
    set_hotkey_failed(!ok);
}

/// 読み込み済みの設定でホットキーを登録する。
/// 戻り値が false なら他のアプリに取られている (Win+W など) 。
pub fn register_hotkey_from_config(hwnd: HWND) -> bool {
    STATE.with(|s| match s.borrow().as_ref() {
        Some(state) => {
            trigger::register_hotkey(hwnd, &state.config.settings.trigger.hotkey).is_ok()
        }
        None => false,
    })
}

/// 現在のホットキー設定。診断表示に使う。
pub fn hotkey_spec() -> String {
    STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| st.config.settings.trigger.hotkey.clone())
            .unwrap_or_default()
    })
}

/// 読み込み済みの設定の項目数。診断表示に使う。
pub fn item_count() -> usize {
    STATE.with(|s| s.borrow().as_ref().map_or(0, |st| st.config.items.len()))
}

/// メニューに登録された実行可能項目の数。診断表示に使う。
pub fn action_count() -> usize {
    STATE.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|st| st.menu.as_ref())
            .map_or(0, |m| m.action_count())
    })
}

/// 組み立て済みメニューの全項目を「表示名 → 解決済みパス」で返す。
/// 診断用。パスが解決できなかった項目はメニューでグレー表示になる。
pub fn dump_actions() -> Vec<(usize, String, String)> {
    STATE.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|st| st.menu.as_ref())
            .map(|m| m.dump())
            .unwrap_or_default()
    })
}

/// ホットキーが取れなかったことをトレイメニューで知らせるために保持する。
pub fn set_hotkey_failed(failed: bool) {
    STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            state.hotkey_failed = failed;
        }
    });
}

/// トレイに常駐する。戻り値は受け口ウィンドウのハンドル。
pub fn install() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;

        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        // WS_EX_TOOLWINDOW: Alt+Tab とタスクバーに出さない。
        // WS_VISIBLE を立てないので画面にも現れない。
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("waypoint"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
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
            let event = (lparam.0 as u32) & 0xffff;
            match event {
                // 左クリックでランチャーメニュー (FR-1.3)
                WM_LBUTTONUP => show_launcher_at_cursor(hwnd),
                WM_RBUTTONUP => show_tray_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        // フックまたは二重起動からの要求 (R-4: 構築はここで行う)
        WM_TRIGGER_MENU => {
            let at = POINT {
                x: wparam.0 as i32,
                y: lparam.0 as i32,
            };
            // 座標が無い (二重起動からの通知) 場合はカーソル位置
            if at.x == 0 && at.y == 0 {
                show_launcher_at_cursor(hwnd);
            } else {
                show_launcher(hwnd, at, trigger::origin_window());
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            show_launcher_at_cursor(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => LRESULT(0),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn show_launcher_at_cursor(hwnd: HWND) {
    // ホットキー経由では元ウィンドウ = 現在の最前面
    let origin = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    show_launcher(hwnd, cursor_pos(), Some(origin));
}

/// ランチャーのメニューを表示し、選ばれた項目を実行する。
fn show_launcher(hwnd: HWND, at: POINT, origin: Option<HWND>) {
    let action = STATE.with(|s| {
        let state = s.borrow();
        let state = state.as_ref()?;
        let menu = state.menu.as_ref()?;
        menu.track(hwnd, at).cloned()
    });

    if let Some(action) = action {
        let _ = shell::open(&action.path, action.open, origin);
    }
}

/// トレイの右クリックメニュー (FR-8.2) 。
fn show_tray_menu(hwnd: HWND) {
    let selected = unsafe { track_tray_menu(hwnd) };
    match selected {
        ID_SETTINGS => open_config_in_editor(),
        ID_RELOAD => reload(hwnd),
        ID_AUTOSTART => {
            let now = crate::autostart::is_enabled();
            let _ = crate::autostart::set_enabled(!now);
        }
        ID_EXIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

/// 管理画面は未実装のため、当面は設定ファイルを既定のエディタで開く。
fn open_config_in_editor() {
    if let Some(path) = crate::config::config_path() {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", &path.to_string_lossy()])
            .spawn();
    }
}

unsafe fn track_tray_menu(hwnd: HWND) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let built = build_tray_items(menu);
        let pos = cursor_pos();

        let selected = if built.is_ok() {
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

unsafe fn build_tray_items(menu: HMENU) -> Result<()> {
    unsafe {
        // 異常があれば先頭に出す (FR-7.4)
        let mut warnings: Vec<String> = Vec::new();
        STATE.with(|s| {
            if let Some(st) = s.borrow().as_ref() {
                if let Some(e) = &st.load_error {
                    warnings.push(e.clone());
                }
                if st.hotkey_failed {
                    // 他アプリに取られている。無言で効かないと原因が分からない
                    warnings.push(format!(
                        "hotkey \"{}\" is taken by another app",
                        st.config.settings.trigger.hotkey
                    ));
                }
            }
        });
        if !warnings.is_empty() {
            for w in &warnings {
                let label = HSTRING::from(format!("⚠ {w}"));
                AppendMenuW(
                    menu,
                    MF_STRING
                        | windows::Win32::UI::WindowsAndMessaging::MF_DISABLED
                        | windows::Win32::UI::WindowsAndMessaging::MF_GRAYED,
                    0,
                    PCWSTR(label.as_ptr()),
                )?;
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        }

        AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Open config file"))?;
        AppendMenuW(menu, MF_STRING, ID_RELOAD, w!("Reload config"))?;

        let autostart_flags = if crate::autostart::is_enabled() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        AppendMenuW(
            menu,
            autostart_flags,
            ID_AUTOSTART,
            w!("Start with Windows"),
        )?;

        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"))?;
        Ok(())
    }
}

/// 中ボタンのトリガーを受け付けてよいか (FR-1.4 / R-1) 。
pub fn middle_click_allowed() -> bool {
    STATE.with(|s| {
        let state = s.borrow();
        let Some(state) = state.as_ref() else {
            return false;
        };
        if !state.config.settings.trigger.middle_click {
            return false;
        }
        // ブラウザ等ではオートスクロールを壊さないよう見送る
        match process::foreground_process_name() {
            Some(name) => {
                !process::is_excluded(&name, &state.config.settings.trigger.excluded_processes)
            }
            None => true,
        }
    })
}
