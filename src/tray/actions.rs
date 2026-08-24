//! メニュー選択後のアクション実行と、トレイ右クリックメニュー (FR-8.2) 。

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DestroyWindow, HMENU, MENUITEMINFOW, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MIIM_BITMAP, SetForegroundWindow, SetMenuItemInfoW, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::menu::{Action, Selection};
use crate::quick_launch;
use crate::quick_launch_window;
use crate::shell;

use super::window::cursor_pos;
use super::{
    ICON_CLOSE, ICON_RELOAD, ID_AUTOSTART, ID_AZURE_REFRESH, ID_EXIT, ID_RELOAD, ID_SETTINGS,
    WM_DYNAMIC_REFRESHED, refresh_azure_devops, reload, with_state,
};

pub(crate) fn show_launcher_at_cursor(hwnd: HWND) {
    // ホットキー経由では元ウィンドウ = 現在の最前面
    let origin = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    show_launcher(hwnd, cursor_pos(), Some(origin));
}

/// ランチャーのメニューを表示し、選ばれた項目を実行する。
pub(crate) fn show_launcher(hwnd: HWND, at: POINT, origin: Option<HWND>) {
    let selection = with_state(|s| {
        let state = s.borrow();
        let state = state.as_ref()?;
        let menu = state.menu.as_ref()?;
        menu.track(hwnd, at)
    });

    match selection {
        Some(Selection::Action(Action::Open { path, open })) => {
            let _ = shell::open(&path, open, origin);
            refresh_dynamic(hwnd);
        }
        Some(Selection::Action(Action::ActivateWindow { hwnd: target })) => {
            shell::activate_window(HWND(target as *mut _));
            refresh_dynamic(hwnd);
        }
        Some(Selection::Action(Action::OpenShell { target })) => {
            let _ = shell::open_shell_item(&target);
            refresh_dynamic(hwnd);
        }
        Some(Selection::Settings) => open_config_in_editor(),
        Some(Selection::Reload) => reload(hwnd),
        Some(Selection::Close) | None => refresh_dynamic(hwnd),
    }
}

/// 保持済みの設定と動的データのままメニューだけ組み直す。
///
/// テーマ変更のようにアイコンだけ作り直したい場合に使う。
/// Recent / Frequent の再列挙は不要なので `refresh_dynamic` は呼ばない。
pub(crate) fn rebuild_menu() {
    with_state(|s| {
        let mut state = s.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.menu = crate::menu::build(&state.config, &state.dynamic).ok();
    });
}

/// Quick Launch の `Ctrl+Shift+Enter` で選択した候補を config の
/// ルートメニュー末尾へ追加保存する。同じパスが既にあれば何もしない
/// (`Config::add_item_if_new`)。保存後はトレイメニューと Quick Launch の
/// 検索インデックスを組み直し、その場で見えるようにする。
pub(crate) fn add_entry_to_favorites(entry: quick_launch::Entry) {
    let Some(item) = entry.to_item() else {
        return;
    };
    let dynamic = with_state(|s| {
        let mut state = s.borrow_mut();
        let state = state.as_mut()?;
        if !state.config.add_item_if_new(item) {
            return None;
        }
        if let Err(e) = crate::config::save(&state.config) {
            crate::panic_log::record(&format!("failed to save added favorite: {e}"));
            return None;
        }
        state.menu = crate::menu::build(&state.config, &state.dynamic).ok();
        Some(state.dynamic.clone())
    });
    if let Some(dynamic) = dynamic {
        with_state(|s| {
            let state = s.borrow();
            if let Some(state) = state.as_ref() {
                quick_launch_window::configure(&state.config, &dynamic);
            }
        });
    }
}

/// メニューが閉じた後に列挙し、次回表示用キャッシュを入れ替える。
///
/// Recent フォルダの COM 解決が数十 ms かかる (実測) ため、UI スレッドを
/// 塞がないようバックグラウンドスレッドで行う。結果は `WM_DYNAMIC_REFRESHED`
/// で受け取ってから反映する (`handle_dynamic_refreshed`)。
pub(crate) fn refresh_dynamic(hwnd: HWND) {
    crate::dynamic::refresh_async(hwnd, WM_DYNAMIC_REFRESHED);
}

/// `WM_DYNAMIC_REFRESHED` を受けて、列挙結果をメニューと Quick Launch へ反映する。
pub(crate) fn handle_dynamic_refreshed() {
    let Some(dynamic) = crate::dynamic::take_refreshed() else {
        return;
    };
    with_state(|s| {
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
pub(crate) fn show_tray_menu(hwnd: HWND) {
    let selected = unsafe { track_tray_menu(hwnd) };
    match selected {
        ID_SETTINGS => open_config_in_editor(),
        ID_RELOAD => reload(hwnd),
        ID_AZURE_REFRESH => refresh_azure_devops(hwnd),
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

pub(crate) fn open_config_in_editor() {
    open_settings(None);
}

pub(crate) fn open_settings(argument: Option<&str>) {
    if let Ok(exe) = std::env::current_exe() {
        let editor = exe.with_file_name("waypoint-settings.exe");
        let mut command = std::process::Command::new(editor);
        if let Some(argument) = argument {
            command.arg(argument);
        }
        let _ = command.spawn();
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
        with_state(|s| {
            if let Some(st) = s.borrow().as_ref() {
                if let Some(e) = &st.load_error {
                    warnings.push(e.clone());
                }
                if st.hotkey_failed {
                    // 他アプリに取られただけならフックで横取りする (FR-1.2.1) 。
                    // ここまで来るのは指定が不正な場合。無言で効かないと原因が分からない
                    warnings.push(format!(
                        "hotkey \"{}\" could not be registered",
                        st.config.settings.trigger.hotkey
                    ));
                }
                if st.quick_launch_hotkey_failed {
                    warnings.push(format!(
                        "Quick Launch hotkey \"{}\" could not be registered",
                        st.config.settings.quick_launch.hotkey
                    ));
                }
                let azure = crate::azure_devops::cache_status(
                    &st.config.settings.quick_launch.azure_devops,
                );
                if azure.refresh_in_progress {
                    warnings.push("Azure DevOps refresh in progress".to_string());
                } else if azure.failed_projects > 0 {
                    warnings.push(format!(
                        "Azure DevOps refresh failed for {} project(s): {}",
                        azure.failed_projects,
                        azure.last_error.unwrap_or_else(|| "see log".to_string())
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
        AppendMenuW(
            menu,
            MF_STRING,
            ID_AZURE_REFRESH,
            w!("Refresh Azure DevOps"),
        )?;

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
