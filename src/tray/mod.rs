//! タスクトレイ常駐と、アプリ全体のメッセージ処理。
//!
//! ウィンドウは見せないが、`HWND_MESSAGE` の子ではなく
//! 「非表示の通常ウィンドウ」にしている。理由は 2 つ:
//!
//! - `TrackPopupMenuEx` は所有ウィンドウを前面化できる必要がある (R-2) 。
//!   メッセージ専用ウィンドウは前面化できない。
//! - 二重起動時に `FindWindowW` で先行プロセスを探す (FR-8.3) 。
//!   メッセージ専用ウィンドウは列挙・検索の対象外になる。

mod actions;
mod window;

use std::cell::RefCell;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, FindWindowW, IDI_APPLICATION, LoadIconW, PostMessageW, RegisterClassW,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, Result, w};

use crate::config::{Config, LoadOutcome};
use crate::menu::BuiltMenu;
use crate::process;
use crate::quick_launch_window;
use crate::trigger::{self, Registration};

use window::wnd_proc;

/// トレイアイコンからの通知。WM_APP 以降はアプリが自由に使える。
pub(crate) const WM_TRAY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
pub(crate) const WM_RELOAD_CONFIG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;
pub const WM_AZURE_DEVOPS_REFRESHED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 8;
/// `dynamic::refresh_async` の完了通知 (Recent/Frequent/Windows の再列挙)。
pub const WM_DYNAMIC_REFRESHED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;
const TRAY_UID: u32 = 1;

// トレイの右クリックメニューの項目 ID
pub(crate) const ID_SETTINGS: usize = 9001;
pub(crate) const ID_RELOAD: usize = 9002;
pub(crate) const ID_AUTOSTART: usize = 9003;
pub(crate) const ID_EXIT: usize = 9004;
pub(crate) const ID_AZURE_REFRESH: usize = 9005;
pub(crate) const ICON_RELOAD: &[u8] = include_bytes!("../../assets/menu/reload.png");
pub(crate) const ICON_CLOSE: &[u8] = include_bytes!("../../assets/menu/close.png");

pub const CLASS_NAME: PCWSTR = w!("WaypointMessageWindow");

thread_local! {
    /// 現在の設定と、そこから組み立てたメニュー。
    /// 設定を再読み込みしたら両方入れ替える。
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

pub(crate) struct AppState {
    pub(crate) config: Config,
    pub(crate) dynamic: crate::dynamic::Menus,
    pub(crate) menu: Option<BuiltMenu>,
    /// 設定の読み込みに失敗した理由。空メニューで起動した場合に入る。
    pub(crate) load_error: Option<String>,
    /// ホットキーが他アプリに取られていた場合に立つ。
    pub(crate) hotkey_failed: bool,
    pub(crate) quick_launch_hotkey_failed: bool,
}

/// `STATE` へアクセスするヘルパー。モジュール分割後も他ファイルから
/// thread_local の中身を読み書きできるようにする。
pub(crate) fn with_state<R>(f: impl FnOnce(&RefCell<Option<AppState>>) -> R) -> R {
    STATE.with(f)
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
    let reg = register_hotkey_from_config(hwnd);
    set_hotkey_failed(!reg.is_active());
    let quick_reg = register_quick_launch_hotkey_from_config(hwnd);
    set_quick_launch_hotkey_failed(!quick_reg.is_active());
    refresh_azure_devops(hwnd);
}

/// Azure DevOps のキャッシュ同期を開始する。完了後の通知で Quick Launch の
/// 検索インデックスだけを差し替えるため、メニュー表示経路へは入らない。
pub fn refresh_azure_devops(hwnd: HWND) {
    STATE.with(|state| {
        let state = state.borrow();
        let Some(state) = state.as_ref() else {
            return;
        };
        let _ = crate::azure_devops::prune_cache(&state.config.settings.quick_launch.azure_devops);
        let _ = crate::azure_devops::refresh_async(
            state.config.settings.quick_launch.azure_devops.clone(),
            hwnd,
            WM_AZURE_DEVOPS_REFRESHED,
        );
    });
}

/// 読み込み済みの設定でホットキーを登録する。
/// 他アプリに取られていた場合はフックで横取りする (FR-1.2.1) 。
pub fn register_hotkey_from_config(hwnd: HWND) -> Registration {
    STATE.with(|s| match s.borrow().as_ref() {
        Some(state) => trigger::register_hotkey(hwnd, &state.config.settings.trigger.hotkey),
        None => Registration::Failed,
    })
}

pub fn register_quick_launch_hotkey_from_config(hwnd: HWND) -> Registration {
    STATE.with(|s| match s.borrow().as_ref() {
        Some(state) => {
            trigger::register_quick_launch_hotkey(hwnd, &state.config.settings.quick_launch.hotkey)
        }
        None => Registration::Failed,
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

/// 受け口ウィンドウだけを作る。トレイアイコンはまだ出さない。
///
/// ホットキー登録には既にこのウィンドウが要る。アイコンが見えてから
/// ホットキーが使えるまでの間にユーザーが触れる隙を作らないよう、
/// 先にウィンドウだけ作って呼び出し側で設定読込・ホットキー登録を
/// 済ませ、その後で `show_icon` を呼ぶ運用にする (main.rs 参照)。
pub fn create_window() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let icon_resource = PCWSTR(std::ptr::without_provenance(1));
        let app_icon = LoadIconW(Some(instance.into()), icon_resource)
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;

        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hIcon: app_icon,
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

        Ok(hwnd)
    }
}

/// トレイアイコンを表示する。`create_window` で作ったハンドルを渡す。
pub fn show_icon(hwnd: HWND) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let icon_resource = PCWSTR(std::ptr::without_provenance(1));
        let app_icon = LoadIconW(Some(instance.into()), icon_resource)
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;

        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: app_icon,
            ..Default::default()
        };
        write_tip(&mut data.szTip, "waypoint");

        Shell_NotifyIconW(NIM_ADD, &data).ok()?;

        Ok(())
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
