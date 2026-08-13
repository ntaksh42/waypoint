//! トリガー: マウス中ボタンの低レベルフックとグローバルホットキー。
//!
//! # フック内で仕事をしないこと
//!
//! 低レベルフックの応答が `LowLevelHooksTimeout` (既定 300ms) を超えると、
//! Windows はフックを**通知なく解除する** (R-4) 。以後トリガーが効かなくなり
//! 再現しづらい不具合になるため、フック内では「メニューを出すか」の判定だけ
//! 行い、実際の構築と表示は `PostMessage` した先で行う。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_MOUSE_LL, WM_APP, WM_MBUTTONDOWN, WM_MBUTTONUP,
};
use windows::core::Result;

/// フックが「メニューを出せ」と伝えるためのメッセージ。
pub const WM_TRIGGER_MENU: u32 = WM_APP + 2;

/// ホットキーの識別子。
pub const HOTKEY_ID: i32 = 1;
pub const QUICK_LAUNCH_HOTKEY_ID: i32 = 2;

/// ドラッグとみなす移動量 (ピクセル) 。これを超えたらメニューを出さない。
const DRAG_THRESHOLD: i32 = 5;

thread_local! {
    /// フックの通知先。フックプロシージャは状態を持てないので
    /// スレッドローカルに置く。
    static TARGET: Cell<isize> = const { Cell::new(0) };
    /// 中ボタンを押した位置。離した位置と比べてドラッグを判定する (FR-1.5) 。
    static PRESS_AT: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// 中ボタンを押した時点で最前面だったウィンドウ (FR-1.6) 。
    static ORIGIN: Cell<isize> = const { Cell::new(0) };
}

/// 直前のトリガーで保持した元ウィンドウ。`reuse` の対象になる。
pub fn origin_window() -> Option<HWND> {
    let raw = ORIGIN.with(|o| o.get());
    (raw != 0).then_some(HWND(raw as *mut _))
}

/// 中ボタンのフックを張る。戻り値は解除用のハンドル。
pub fn install_mouse_hook(target: HWND) -> Result<HHOOK> {
    TARGET.with(|t| t.set(target.0 as isize));
    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) }
}

pub fn remove_mouse_hook(hook: HHOOK) {
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // code < 0 のときは処理せず次へ回すのが規約
    if code >= 0 {
        let msg = wparam.0 as u32;
        if msg == WM_MBUTTONDOWN || msg == WM_MBUTTONUP {
            // 生ポインタ参照は最小範囲に閉じる
            let pt = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };

            if msg == WM_MBUTTONDOWN {
                PRESS_AT.with(|p| p.set((pt.x, pt.y)));
                ORIGIN.with(|o| {
                    let fg =
                        unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                    o.set(fg.0 as isize);
                });
            } else if should_open(pt) {
                // ここで消費し、アプリ側には中ボタンを渡さない
                notify(pt);
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 離した位置が押した位置から離れていればドラッグ (FR-1.5) 。
/// ドラッグならメニューを出さず、アプリ側にそのまま通す。
fn should_open(release: POINT) -> bool {
    let (px, py) = PRESS_AT.with(|p| p.get());
    (release.x - px).abs() <= DRAG_THRESHOLD && (release.y - py).abs() <= DRAG_THRESHOLD
}

/// フック内では判定だけ。構築と表示はメッセージを投げた先で行う (R-4) 。
fn notify(pt: POINT) {
    let target = TARGET.with(|t| t.get());
    if target == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            Some(HWND(target as *mut _)),
            WM_TRIGGER_MENU,
            WPARAM(pt.x as usize),
            LPARAM(pt.y as isize),
        );
    }
}

/// `"Win+W"` のような文字列を修飾キーと仮想キーコードに分解する。
pub fn parse_hotkey(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut key = None;

    for part in spec.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "win" => mods |= MOD_WIN,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "" => return None,
            other => {
                key = Some(virtual_key(other)?);
            }
        }
    }
    key.map(|k| (mods, k))
}

/// キー名を仮想キーコードに変換する。英数字とファンクションキーのみ対応。
fn virtual_key(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0].to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }
    if let Some(n) = name.strip_prefix('f')
        && let Ok(n) = n.parse::<u32>()
        && (1..=24).contains(&n)
    {
        // VK_F1 = 0x70
        return Some(0x70 + n - 1);
    }
    None
}

/// ホットキーを登録する。既に使われている場合は Err。
pub fn register_hotkey(target: HWND, spec: &str) -> Result<()> {
    register_hotkey_with_id(target, HOTKEY_ID, spec)
}

pub fn register_quick_launch_hotkey(target: HWND, spec: &str) -> Result<()> {
    register_hotkey_with_id(target, QUICK_LAUNCH_HOTKEY_ID, spec)
}

fn register_hotkey_with_id(target: HWND, id: i32, spec: &str) -> Result<()> {
    let Some((mods, vk)) = parse_hotkey(spec) else {
        return Err(windows::core::Error::empty());
    };
    unsafe { RegisterHotKey(Some(target), id, mods, vk) }
}

pub fn unregister_hotkeys(target: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(target), HOTKEY_ID);
        let _ = UnregisterHotKey(Some(target), QUICK_LAUNCH_HOTKEY_ID);
    }
}
