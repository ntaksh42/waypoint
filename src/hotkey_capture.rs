//! ホットキーを実際のキー入力から記録する (FR-6.8.1) 。
//!
//! 設定画面で使う。記録中は `WH_KEYBOARD_LL` で打鍵をすべて握り潰すため、
//! `Win+W` のように OS が予約している組み合わせもそのまま記録でき、
//! 記録中に Widgets やスタートメニューが開くこともない。
//!
//! フックを張ったスレッドがメッセージを回している必要がある。設定画面は
//! eframe がメッセージループを持つので、UI スレッドから呼ぶこと。

use std::cell::Cell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
    VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::trigger::format_hotkey;

/// 記録の結果。
pub enum Captured {
    /// キーの組み合わせが確定した。
    Spec(String),
    /// `Esc` で取り消した。
    Cancelled,
    /// ホットキーに使えないキーだった (記号キーなど) 。
    Unsupported,
}

thread_local! {
    static HOOK: Cell<isize> = const { Cell::new(0) };
    /// 確定した打鍵。フックプロシージャから UI へ渡す唯一の口。
    static RESULT: Cell<Option<Outcome>> = const { Cell::new(None) };
}

/// フックプロシージャが書き込める形。`String` は Cell に置けないので
/// 仮想キーコードのまま運び、文字列化は UI 側で行う。
#[derive(Clone, Copy)]
enum Outcome {
    Key(u32, HOT_KEY_MODIFIERS),
    Cancelled,
}

/// 記録を開始する。フックを張れなければ `false`。
///
/// 呼び出し側は戻り値を見ること。張れていないのに記録中の表示にすると、
/// いくら打鍵しても何も起きない画面になる。
pub fn start() -> bool {
    RESULT.with(|r| r.set(None));
    if HOOK.with(|h| h.get()) != 0 {
        return true;
    }
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0) } {
        Ok(hook) => {
            HOOK.with(|h| h.set(hook.0 as isize));
            true
        }
        Err(e) => {
            crate::panic_log::record(&format!("hotkey capture: SetWindowsHookExW failed: {e}"));
            false
        }
    }
}

/// 記録を終える。フックを張ったままにしない。
pub fn stop() {
    let raw = HOOK.with(|h| h.replace(0));
    if raw != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
        }
    }
    RESULT.with(|r| r.set(None));
}

pub fn is_active() -> bool {
    HOOK.with(|h| h.get()) != 0
}

/// 確定していれば結果を返す。UI から毎フレーム呼ぶ。
/// 何か返した時点で記録は終わる。
pub fn poll() -> Option<Captured> {
    let outcome = RESULT.with(|r| r.take())?;
    stop();
    Some(match outcome {
        Outcome::Cancelled => Captured::Cancelled,
        Outcome::Key(vk, mods) => match format_hotkey(mods, vk) {
            Some(spec) => Captured::Spec(spec),
            None => Captured::Unsupported,
        },
    })
}

/// 記録中はすべての打鍵を消費する。ここでは判定だけ行い、文字列化と
/// 画面の更新は `poll` を呼ぶ UI 側で行う (R-4) 。
unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    // 生ポインタ参照は最小範囲に閉じる
    let vk = unsafe { (*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode };
    let msg = wparam.0 as u32;

    if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && RESULT.with(|r| r.get().is_none()) {
        if vk == VK_ESCAPE.0 as u32 {
            RESULT.with(|r| r.set(Some(Outcome::Cancelled)));
        } else if !is_modifier(vk) {
            // 修飾キー単独では確定させない。基準キーが来た時点で締める
            RESULT.with(|r| r.set(Some(Outcome::Key(vk, current_modifiers()))));
        }
    }
    // 修飾キーの解放まで含めて何も外へ出さない。
    // 押下だけ握り潰すと、スタートメニューが開く余地が残る
    LRESULT(1)
}

fn is_modifier(vk: u32) -> bool {
    const VK_LSHIFT: u32 = 0xA0;
    const VK_RMENU: u32 = 0xA5;
    matches!(vk, x if x == VK_LWIN.0 as u32 || x == VK_RWIN.0 as u32)
        || matches!(vk, x if x == VK_SHIFT.0 as u32 || x == VK_CONTROL.0 as u32 || x == VK_MENU.0 as u32)
        // 左右個別の仮想キー (VK_LSHIFT..VK_RMENU) もまとめて弾く
        || (VK_LSHIFT..=VK_RMENU).contains(&vk)
}

/// いま押されている修飾キー。
fn current_modifiers() -> HOT_KEY_MODIFIERS {
    let down = |vk: VIRTUAL_KEY| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
    let mut mods = HOT_KEY_MODIFIERS(0);
    if down(VK_LWIN) || down(VK_RWIN) {
        mods |= MOD_WIN;
    }
    if down(VK_CONTROL) {
        mods |= MOD_CONTROL;
    }
    if down(VK_MENU) {
        mods |= MOD_ALT;
    }
    if down(VK_SHIFT) {
        mods |= MOD_SHIFT;
    }
    mods
}
