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
    FindWindowW, GetCursorPos, HMENU, IDI_APPLICATION, LoadIconW, MENUITEMINFOW, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MIIM_BITMAP, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetMenuItemInfoW, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_APP, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WM_THEMECHANGED, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::config::{Config, LoadOutcome};
use crate::menu::{Action, BuiltMenu, Selection};
use crate::process;
use crate::quick_launch_window::{self, WM_QUICK_LAUNCH_EXECUTE};
use crate::shell;
use crate::trigger::{self, WM_TRIGGER_MENU};

/// トレイアイコンからの通知。WM_APP 以降はアプリが自由に使える。
const WM_TRAY: u32 = WM_APP + 1;
const WM_RELOAD_CONFIG: u32 = WM_APP + 3;
const TRAY_UID: u32 = 1;

// トレイの右クリックメニューの項目 ID
const ID_SETTINGS: usize = 9001;
const ID_RELOAD: usize = 9002;
const ID_AUTOSTART: usize = 9003;
const ID_EXIT: usize = 9004;
const ICON_RELOAD: &[u8] = include_bytes!("../assets/menu/reload.png");
const ICON_CLOSE: &[u8] = include_bytes!("../assets/menu/close.png");

pub const CLASS_NAME: PCWSTR = w!("WaypointMessageWindow");

thread_local! {
    /// 現在の設定と、そこから組み立てたメニュー。
    /// 設定を再読み込みしたら両方入れ替える。
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

struct AppState {
    config: Config,
    dynamic: crate::dynamic::Menus,
    menu: Option<BuiltMenu>,
    /// 設定の読み込みに失敗した理由。空メニューで起動した場合に入る。
    load_error: Option<String>,
    /// ホットキーが他アプリに取られていた場合に立つ。
    hotkey_failed: bool,
    quick_launch_hotkey_failed: bool,
}

/// 設定を読み込み、メニューを構築して保持する。
pub fn load_state() {
    let (config, load_error) = match crate::config::load() {
        LoadOutcome::Loaded(c) => {
            crate::panic_log::record(&format!("config loaded: {} items", c.items.len()));
            (c, None)
        }
        LoadOutcome::Created(c) => {
            crate::panic_log::record("config not found; wrote defaults");
            (c, None)
        }
        // 壊れた設定でも起動は続ける (FR-7.4)
        LoadOutcome::Failed(e) => {
            crate::panic_log::record(&format!("config load failed: {e}"));
            (Config::default(), Some(e))
        }
    };
    // 解決できない変数を含む項目を残す。メニューではグレー表示に
    // なるだけで理由が分からない (FR-5.4)
    for (name, path) in crate::config::unresolved_items(&config) {
        crate::panic_log::record(&format!("unresolved variable in \"{name}\": {path}"));
    }
    let dynamic = crate::dynamic::refresh();
    let menu = crate::menu::build(&config, &dynamic).ok();
    quick_launch_window::configure(&config, &dynamic);
    STATE.with(|s| {
        *s.borrow_mut() = Some(AppState {
            config,
            dynamic,
            menu,
            load_error,
            hotkey_failed: false,
            quick_launch_hotkey_failed: false,
        })
    });
}

/// 設定を読み直してメニューを組み立て直す (FR-8.2) 。
pub fn reload(hwnd: HWND) {
    // ホットキーが変わっているかもしれないので張り直す
    trigger::unregister_hotkeys(hwnd);
    // 項目のパスやアイコン指定が変わっている可能性がある。
    // 古いビットマップを使い回さないよう捨ててから組み直す
    crate::icon::clear_cache();
    load_state();
    let ok = register_hotkey_from_config(hwnd);
    set_hotkey_failed(!ok);
    let quick_ok = register_quick_launch_hotkey_from_config(hwnd);
    set_quick_launch_hotkey_failed(!quick_ok);
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

pub fn register_quick_launch_hotkey_from_config(hwnd: HWND) -> bool {
    STATE.with(|s| match s.borrow().as_ref() {
        Some(state) => {
            trigger::register_quick_launch_hotkey(hwnd, &state.config.settings.quick_launch.hotkey)
                .is_ok()
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

pub fn set_quick_launch_hotkey_failed(failed: bool) {
    STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            state.quick_launch_hotkey_failed = failed;
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

/// Win32 から呼ばれる入口。
///
/// `extern "system"` は unwind できず、panic すると abort する。
/// ここが落ちると常駐部ごと消えてトレイアイコンも残らないため、
/// 捕まえてログに残し、既定処理へ流して動作を続ける。
extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match std::panic::catch_unwind(|| dispatch(hwnd, msg, wparam, lparam)) {
        Ok(result) => result,
        Err(_) => {
            crate::panic_log::record(&format!(
                "tray wnd_proc panicked on message 0x{msg:04x}; recovered"
            ));
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            if wparam.0 as i32 == trigger::QUICK_LAUNCH_HOTKEY_ID {
                let origin =
                    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                let _ = quick_launch_window::show(hwnd, Some(origin));
            } else {
                show_launcher_at_cursor(hwnd);
            }
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_EXECUTE => {
            if let Some((entry, mode, origin)) = quick_launch_window::take_pending() {
                let _ = shell::open(&entry.path, mode, origin);
                refresh_dynamic();
            }
            LRESULT(0)
        }
        WM_RELOAD_CONFIG => {
            reload(hwnd);
            LRESULT(0)
        }
        // ダーク / ライトの切り替えでアイコンの見え方が変わる。
        // 古いビットマップを捨て、次回表示で引き直す
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            crate::icon::clear_cache();
            crate::theme::enable_dark_menus();
            rebuild_menu();
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
    let selection = STATE.with(|s| {
        let state = s.borrow();
        let state = state.as_ref()?;
        let menu = state.menu.as_ref()?;
        menu.track(hwnd, at)
    });

    match selection {
        Some(Selection::Action(Action::Open { path, open })) => {
            let _ = shell::open(&path, open, origin);
            refresh_dynamic();
        }
        Some(Selection::Action(Action::ActivateWindow { hwnd })) => {
            shell::activate_window(HWND(hwnd as *mut _));
            refresh_dynamic();
        }
        Some(Selection::Action(Action::OpenShell { target })) => {
            let _ = shell::open_shell_item(&target);
            refresh_dynamic();
        }
        Some(Selection::AddSpecialFolder) => open_settings(Some("--add-special-folder")),
        Some(Selection::Settings) => open_config_in_editor(),
        Some(Selection::Reload) => reload(hwnd),
        Some(Selection::Close) | None => refresh_dynamic(),
    }
}

/// 保持済みの設定と動的データのままメニューだけ組み直す。
///
/// テーマ変更のようにアイコンだけ作り直したい場合に使う。
/// Recent / Frequent の再列挙は不要なので `refresh_dynamic` は呼ばない。
fn rebuild_menu() {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.menu = crate::menu::build(&state.config, &state.dynamic).ok();
    });
}

/// メニューが閉じた後に列挙し、次回表示用キャッシュを入れ替える。
fn refresh_dynamic() {
    let dynamic = crate::dynamic::refresh();
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.menu = crate::menu::build(&state.config, &dynamic).ok();
        quick_launch_window::configure(&state.config, &dynamic);
        state.dynamic = dynamic;
    });
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

fn open_config_in_editor() {
    open_settings(None);
}

fn open_settings(argument: Option<&str>) {
    if let Ok(exe) = std::env::current_exe() {
        let editor = exe.with_file_name("waypoint-settings.exe");
        let mut command = std::process::Command::new(editor);
        if let Some(argument) = argument {
            command.arg(argument);
        }
        let _ = command.spawn();
    }
}

/// 管理画面から常駐部へ設定の再読み込みを通知する。
pub fn signal_reload() -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(CLASS_NAME, None) else {
            return false;
        };
        PostMessageW(
            Some(hwnd),
            WM_RELOAD_CONFIG,
            Default::default(),
            Default::default(),
        )
        .is_ok()
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
                if st.quick_launch_hotkey_failed {
                    warnings.push(format!(
                        "Quick Launch hotkey \"{}\" is taken by another app",
                        st.config.settings.quick_launch.hotkey
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

        AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings..."))?;
        // ランチャーメニュー側と同じ歯車で揃える
        if let Some(bitmap) = crate::icon::bitmap_for_settings() {
            set_tray_bitmap(menu, ID_SETTINGS, bitmap);
        }
        AppendMenuW(menu, MF_STRING, ID_RELOAD, w!("Reload config"))?;
        set_tray_item_icon(menu, ID_RELOAD, "reload", ICON_RELOAD);

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
        if let Ok(exe) = std::env::current_exe()
            && let Some(bitmap) = crate::icon::bitmap_for(exe.to_string_lossy().as_ref())
        {
            set_tray_bitmap(menu, ID_AUTOSTART, bitmap);
        }

        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"))?;
        set_tray_item_icon(menu, ID_EXIT, "close", ICON_CLOSE);
        Ok(())
    }
}

unsafe fn set_tray_item_icon(menu: HMENU, id: usize, key: &str, png: &[u8]) {
    unsafe {
        let Some(bitmap) = crate::icon::bitmap_for_asset(key, png) else {
            return;
        };
        set_tray_bitmap(menu, id, bitmap);
    }
}

unsafe fn set_tray_bitmap(menu: HMENU, id: usize, bitmap: windows::Win32::Graphics::Gdi::HBITMAP) {
    unsafe {
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            hbmpItem: bitmap,
            ..Default::default()
        };
        let _ = SetMenuItemInfoW(menu, id as u32, false, &info);
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
