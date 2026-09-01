//! Win32 メッセージ処理本体 (WndProc) 。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, PostQuitMessage, WM_COMMAND, WM_COPYDATA, WM_DESTROY,
    WM_DRAWITEM, WM_HOTKEY, WM_LBUTTONUP, WM_MEASUREITEM, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_THEMECHANGED, WM_TIMER,
};

use crate::quick_launch;
use crate::quick_launch_history;
use crate::quick_launch_window::{self, WM_QUICK_LAUNCH_ADD_TO_FAVORITES, WM_QUICK_LAUNCH_EXECUTE};
use crate::shell;
use crate::trigger::{self, WM_TRIGGER_MENU};

use super::actions::{
    add_entry_to_favorites, handle_dynamic_refreshed, open_settings, rebuild_menu, refresh_dynamic,
    show_launcher, show_launcher_at_cursor, show_tray_menu,
};
use super::{
    AZURE_FULL_REFRESH_TIMER_ID, WM_AZURE_DEVOPS_REFRESHED, WM_DYNAMIC_REFRESHED, WM_RELOAD_CONFIG,
    WM_TRAY, refresh_azure_devops, reload, with_state,
};

/// Win32 から呼ばれる入口。
///
/// `extern "system"` は unwind できず、panic すると abort する。
/// ここが落ちると常駐部ごと消えてトレイアイコンも残らないため、
/// 捕まえてログに残し、既定処理へ流して動作を続ける。
pub(crate) extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
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
                // config 由来の候補 (showBranch の Git ブランチ名を含む) を
                // 表示のたびに読み直す。COM/SQLite を伴わず軽量なので
                // トリガー経路で呼んでも 50ms 予算を圧迫しない
                // (`Index::refresh_config_items` 参照)。ここで呼ばないと、
                // 直前のフル構築 (起動時・設定保存時・お気に入り昇格時) 時点の
                // ブランチ名のまま古くなる。
                with_state(|state| {
                    let state = state.borrow();
                    if let Some(state) = state.as_ref() {
                        quick_launch_window::configure_config_items(&state.config, &state.dynamic);
                    }
                });
                let _ = quick_launch_window::show(hwnd, Some(origin));
            } else {
                show_launcher_at_cursor(hwnd);
            }
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_EXECUTE => {
            if let Some((entry, origin)) = quick_launch_window::take_pending() {
                // 記録は次回以降の並び順にしか効かない。ディスク flush を待つと
                // そのまま「選んでから開くまで」の遅延になる (実測 18.8ms)
                quick_launch_history::record_async(&entry);
                match entry.action {
                    quick_launch::Action::OpenFolder(mode) => {
                        let _ = shell::open(&entry.path, mode, origin);
                    }
                    quick_launch::Action::FocusWindow(hwnd) => {
                        shell::activate_window(HWND(hwnd as *mut _));
                    }
                    quick_launch::Action::FocusBrowserTab(target) => {
                        let _ = crate::browser_tabs::request_focus(&target);
                    }
                    quick_launch::Action::OpenUrl(url) => {
                        let _ = shell::open_shell_item(&url);
                    }
                    quick_launch::Action::OpenWithDefaultHandler
                    | quick_launch::Action::LaunchApp => {
                        let _ = shell::open_shell_item(&entry.path);
                    }
                    quick_launch::Action::OpenInTerminal => {
                        let _ = shell::open_terminal(&entry.path);
                    }
                    // ReplaceQuery / AzureLiveWorkItemSearch / AzureLivePullRequestSearch /
                    // AzureLivePipelineSearch はウィンドウを閉じずに Quick Launch
                    // 側で完結する (queue_selected 参照)。ここへは実行対象として来ない
                    quick_launch::Action::ReplaceQuery(_)
                    | quick_launch::Action::AzureLiveWorkItemSearch(_)
                    | quick_launch::Action::AzureLivePullRequestSearch { .. }
                    | quick_launch::Action::AzureLivePipelineSearch { .. } => {}
                    quick_launch::Action::AzureSuggestPriorities => {
                        open_settings(Some(crate::azure_devops::AZURE_SUGGEST_ARG));
                    }
                }
                refresh_dynamic(hwnd);
            }
            LRESULT(0)
        }
        WM_QUICK_LAUNCH_ADD_TO_FAVORITES => {
            if let Some(entry) = quick_launch_window::take_pending_add() {
                add_entry_to_favorites(entry);
            }
            LRESULT(0)
        }
        WM_RELOAD_CONFIG => {
            reload(hwnd);
            LRESULT(0)
        }
        WM_AZURE_DEVOPS_REFRESHED => {
            with_state(|state| {
                let state = state.borrow();
                if let Some(state) = state.as_ref() {
                    // Azure の候補だけ差し替える。フル `configure` にすると
                    // 同期のたびにスタートメニューの再スキャンが道連れになる
                    quick_launch_window::configure_azure(&state.config);
                }
            });
            LRESULT(0)
        }
        WM_DYNAMIC_REFRESHED => {
            handle_dynamic_refreshed();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == AZURE_FULL_REFRESH_TIMER_ID => {
            refresh_azure_devops(hwnd);
            LRESULT(0)
        }
        WM_COPYDATA if lparam.0 != 0 => {
            unsafe {
                let copy_data =
                    &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                if copy_data.dwData == crate::browser_tabs::SNAPSHOT_COPYDATA
                    && !copy_data.lpData.is_null()
                    && copy_data.cbData > 0
                    && copy_data.cbData as usize <= crate::browser_tabs::MAX_SNAPSHOT_BYTES
                {
                    // Native Messaging host はこのハンドラから戻ると送信バッファを解放する。
                    // 先に JSON をコピーし、Quick Launch のキャッシュ更新はそのコピーだけで行う。
                    let bytes = std::slice::from_raw_parts(
                        copy_data.lpData.cast::<u8>(),
                        copy_data.cbData as usize,
                    );
                    if let Some((browser, tabs)) = crate::browser_tabs::parse_snapshot(bytes) {
                        quick_launch_window::replace_browser_tabs(browser, tabs);
                    }
                }
            }
            LRESULT(1)
        }
        // ダーク / ライトの切り替えでアイコンの見え方が変わる。
        // 古いビットマップを捨て、次回表示で引き直す。
        //
        // `WM_SETTINGCHANGE` はテーマ以外 (環境変数・ポリシー・ロケール・
        // 電源設定など) でも飛んでくる。全部に反応するとアイコンキャッシュを
        // 捨ててメニューを組み直す処理が毎回走り、UI スレッドで実測 15ms
        // かかる。テーマ変更は lParam が "ImmersiveColorSet" で判別できる
        // ので、それ以外は無視する。
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            if msg == WM_THEMECHANGED || is_immersive_color_set(lparam) {
                crate::icon::clear_cache();
                // メニューフォントとテーマの色も変わる。掴んだまま使わない
                crate::menu_draw::reset_font();
                crate::theme::enable_dark_menus();
                rebuild_menu();
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // オーナードロー項目の採寸と描画 (FR-2.3)
        WM_MEASUREITEM => {
            if crate::menu_draw::measure(wparam, lparam) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_DRAWITEM => {
            if crate::menu_draw::draw(wparam, lparam) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_COMMAND => LRESULT(0),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(crate) fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

/// `WM_SETTINGCHANGE` の lParam がテーマ変更 (`"ImmersiveColorSet"`) を
/// 指しているか。
///
/// lParam は変更されたセクション名を指す UTF-16 文字列 (無い場合は NULL)。
/// ダーク / ライトの切り替えはこの名前で通知される。他の設定変更でも同じ
/// メッセージが飛ぶため、名前を見ないと無関係な変更でメニューを組み直す
/// ことになる。
fn is_immersive_color_set(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // 送り元は別プロセスだが、文字列は共有される形で渡る (Win32 の規約)。
    // NUL 終端までを読む。長さが妥当な範囲に収まらなければ弾く。
    let ptr = lparam.0 as *const u16;
    let mut len = 0usize;
    // "ImmersiveColorSet" は 17 文字。想定より長ければ別の通知なので
    // 途中で打ち切ってよい (全体を読み切る必要が無い)。
    const MAX: usize = 32;
    unsafe {
        while len < MAX && *ptr.add(len) != 0 {
            len += 1;
        }
        if len == 0 || len >= MAX {
            return false;
        }
        let name = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(name) == "ImmersiveColorSet"
    }
}

#[cfg(test)]
mod tests {
    use super::is_immersive_color_set;
    use windows::Win32::Foundation::LPARAM;

    /// UTF-16 の NUL 終端文字列を作り、その先頭を指す LPARAM を返す。
    /// バッファは呼び出し側が保持し続ける必要がある (寿命を借用で縛る)。
    fn lparam_for(buffer: &[u16]) -> LPARAM {
        LPARAM(buffer.as_ptr() as isize)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    #[test]
    fn immersive_color_set_is_recognized() {
        let buffer = wide("ImmersiveColorSet");
        assert!(is_immersive_color_set(lparam_for(&buffer)));
    }

    /// テーマ以外の通知では組み直さない。これが効かないと、無関係な設定変更
    /// のたびにアイコンキャッシュ破棄とメニュー再構築 (実測 15ms) が走る。
    #[test]
    fn other_sections_are_ignored() {
        for name in ["Environment", "Policy", "intl", "WindowsThemeElement"] {
            let buffer = wide(name);
            assert!(
                !is_immersive_color_set(lparam_for(&buffer)),
                "{name} を誤ってテーマ変更として扱っている"
            );
        }
    }

    /// lParam は NULL のことがある (セクション名を伴わない通知)。
    #[test]
    fn null_lparam_is_ignored() {
        assert!(!is_immersive_color_set(LPARAM(0)));
    }

    /// 空文字列と、想定より長い文字列は弾く (打ち切り条件の確認)。
    #[test]
    fn empty_and_overlong_names_are_ignored() {
        let empty = wide("");
        assert!(!is_immersive_color_set(lparam_for(&empty)));

        let overlong = wide(&"A".repeat(64));
        assert!(!is_immersive_color_set(lparam_for(&overlong)));
    }

    /// 前方一致では通さない (`ImmersiveColorSetting` のような別名を弾く)。
    #[test]
    fn prefix_match_is_not_enough() {
        let buffer = wide("ImmersiveColorSetting");
        assert!(!is_immersive_color_set(lparam_for(&buffer)));
    }
}
