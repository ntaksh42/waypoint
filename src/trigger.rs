//! トリガー: マウス中ボタンの低レベルフックとグローバルホットキー。
//!
//! # フック内で仕事をしないこと
//!
//! 低レベルフックの応答が `LowLevelHooksTimeout` (既定 300ms) を超えると、
//! Windows はフックを**通知なく解除する** (R-4) 。以後トリガーが効かなくなり
//! 再現しづらい不具合になるため、フック内では「メニューを出すか」の判定だけ
//! 行い、実際の構築と表示は `PostMessage` した先で行う。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS,
    KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::Result;

/// フックが「メニューを出せ」と伝えるためのメッセージ。
pub const WM_TRIGGER_MENU: u32 = WM_APP + 2;

/// ホットキーの識別子。
pub const HOTKEY_ID: i32 = 1;
pub const QUICK_LAUNCH_HOTKEY_ID: i32 = 2;

/// ドラッグとみなす移動量 (ピクセル) 。これを超えたらメニューを出さない。
const DRAG_THRESHOLD: i32 = 5;

/// 自分が送った打鍵の目印。フックで自分の注入を見分けるために付ける。
/// 値は "WYPT"。
const OWN_INPUT_TAG: usize = 0x5759_5054;

thread_local! {
    /// フックの通知先。フックプロシージャは状態を持てないので
    /// スレッドローカルに置く。
    static TARGET: Cell<isize> = const { Cell::new(0) };
    /// 中ボタンを押した位置。離した位置と比べてドラッグを判定する (FR-1.5) 。
    static PRESS_AT: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// 中ボタンを押した時点で最前面だったウィンドウ (FR-1.6) 。
    static ORIGIN: Cell<isize> = const { Cell::new(0) };
    /// `RegisterHotKey` が失敗し、フックで横取りするホットキー (FR-1.2.1) 。
    static FALLBACKS: RefCell<Vec<Fallback>> = const { RefCell::new(Vec::new()) };
    /// 横取り用のキーボードフック。必要になるまで張らない。
    static KEY_HOOK: Cell<isize> = const { Cell::new(0) };
    /// 押下を握り潰した仮想キー。対になる解放も飲み込む。
    static SWALLOWED: Cell<u32> = const { Cell::new(0) };
}

/// ホットキーをどの手段で受け取っているか。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Registration {
    /// `RegisterHotKey` で取れた。
    Native,
    /// OS 側に先に握られていたので、フックで横取りしている (FR-1.2.1) 。
    Hook,
    /// 指定が不正、またはフックも張れず受け取れない。
    Failed,
}

impl Registration {
    /// ホットキーが実際に効くか。
    pub fn is_active(self) -> bool {
        self != Registration::Failed
    }

    /// 診断表示用の名前。
    pub fn label(self) -> &'static str {
        match self {
            Registration::Native => "native",
            Registration::Hook => "hook",
            Registration::Failed => "failed",
        }
    }
}

/// フックで横取りするホットキー 1 件。
#[derive(Clone, Copy)]
struct Fallback {
    id: i32,
    mods: HOT_KEY_MODIFIERS,
    vk: u32,
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

/// キー名を仮想キーコードに変換する。英数字・ファンクションキー・
/// スペースキーに対応する。
fn virtual_key(name: &str) -> Option<u32> {
    if name.eq_ignore_ascii_case("space") {
        // VK_SPACE
        return Some(0x20);
    }
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

/// 修飾キーと仮想キーコードを `"Win+W"` の形に戻す。`parse_hotkey` の逆。
/// キー入力から設定する経路 (FR-6.8.1) で使う。
pub fn format_hotkey(mods: HOT_KEY_MODIFIERS, vk: u32) -> Option<String> {
    let name = key_name(vk)?;
    let mut out = String::new();
    for (m, label) in [
        (MOD_WIN, "Win"),
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
    ] {
        if (mods & m).0 != 0 {
            out.push_str(label);
            out.push('+');
        }
    }
    out.push_str(&name);
    Some(out)
}

/// 仮想キーコードをキー名に戻す。`virtual_key` が読める範囲だけを返す。
///
/// 英数字の仮想キーコードは大文字と数字の ASCII に一致する。小文字の
/// コード域は別のキーなので、範囲で絞らないと `VK_F5` (0x74) が `t` になる。
fn key_name(vk: u32) -> Option<String> {
    // VK_SPACE
    if vk == 0x20 {
        return Some("Space".to_string());
    }
    // '0'..='9' と 'A'..='Z'
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from_u32(vk).map(|c| c.to_string());
    }
    // VK_F1 = 0x70, VK_F24 = 0x87
    if (0x70..=0x87).contains(&vk) {
        return Some(format!("F{}", vk - 0x70 + 1));
    }
    None
}

/// ホットキーを登録する。
pub fn register_hotkey(target: HWND, spec: &str) -> Registration {
    register_hotkey_with_id(target, HOTKEY_ID, spec)
}

pub fn register_quick_launch_hotkey(target: HWND, spec: &str) -> Registration {
    register_hotkey_with_id(target, QUICK_LAUNCH_HOTKEY_ID, spec)
}

/// `RegisterHotKey` を試し、取られていたらフックへ退避する (FR-1.2.1) 。
///
/// Windows 11 の Widgets は `Win+W` を先に握っており、`RegisterHotKey` は
/// 「既に登録されています」で失敗する。低レベルフックは OS のホットキー処理
/// より手前で打鍵を受け取るので、そこで消費すれば上書きできる。
fn register_hotkey_with_id(target: HWND, id: i32, spec: &str) -> Registration {
    let Some((mods, vk)) = parse_hotkey(spec) else {
        return Registration::Failed;
    };
    if unsafe { RegisterHotKey(Some(target), id, mods, vk) }.is_ok() {
        return Registration::Native;
    }
    if install_keyboard_hook(target).is_err() {
        return Registration::Failed;
    }
    FALLBACKS.with(|f| {
        let mut list = f.borrow_mut();
        list.retain(|x| x.id != id);
        list.push(Fallback { id, mods, vk });
    });
    Registration::Hook
}

pub fn unregister_hotkeys(target: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(target), HOTKEY_ID);
        let _ = UnregisterHotKey(Some(target), QUICK_LAUNCH_HOTKEY_ID);
    }
    FALLBACKS.with(|f| f.borrow_mut().clear());
    remove_keyboard_hook();
}

/// 横取りが必要になった時点で 1 度だけ張る。
fn install_keyboard_hook(target: HWND) -> Result<()> {
    TARGET.with(|t| t.set(target.0 as isize));
    if KEY_HOOK.with(|h| h.get()) != 0 {
        return Ok(());
    }
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0) }?;
    KEY_HOOK.with(|h| h.set(hook.0 as isize));
    Ok(())
}

fn remove_keyboard_hook() {
    let raw = KEY_HOOK.with(|h| h.replace(0));
    if raw != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
        }
    }
}

/// 横取り対象の打鍵だけを消費する。マウスフックと同様、ここでは判定のみ
/// 行い、メニューの構築と表示は `PostMessage` した先で行う (R-4) 。
unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        // 生ポインタ参照は最小範囲に閉じる
        let info = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // 自分で送った打鍵 (スタートメニュー抑止の Ctrl) だけを外す。
        // 合成入力を一律に外すと、キーリマッパー経由の打鍵まで拾えなくなる
        let own = info.dwExtraInfo == OWN_INPUT_TAG;

        if !own && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) {
            if let Some(f) = match_fallback(info.vkCode) {
                SWALLOWED.with(|s| s.set(info.vkCode));
                if (f.mods & MOD_WIN).0 != 0 {
                    break_win_chord();
                }
                post_hotkey(f.id);
                return LRESULT(1);
            }
        } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
            // 押下を握り潰した以上、解放だけをアプリへ渡さない
            let swallowed = SWALLOWED.with(|s| s.get());
            if swallowed != 0 && swallowed == info.vkCode {
                SWALLOWED.with(|s| s.set(0));
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 押されたキーが横取り対象で、修飾キーの状態も一致するか。
///
/// 同じ vk を異なる修飾キーで登録した Fallback が複数あり得る
/// (例: メインホットキーとクイックランチのホットキーが両方ともフック
/// 経路に落ちた場合) 。最初に vk が一致した 1 件だけを見ると、修飾キーが
/// 違う後続の Fallback が一致するはずの入力を取りこぼすため、vk が一致する
/// 全件について修飾キーの一致を確かめる。
fn match_fallback(vk: u32) -> Option<Fallback> {
    FALLBACKS.with(|f| find_matching(f.borrow().iter().copied(), vk, modifiers_match))
}

/// `match_fallback` の本体。実際のキー状態 (`GetAsyncKeyState`) から
/// 切り離してテストできるよう、一致判定を関数として受け取る。
fn find_matching(
    fallbacks: impl Iterator<Item = Fallback>,
    vk: u32,
    modifiers_match: impl Fn(HOT_KEY_MODIFIERS) -> bool,
) -> Option<Fallback> {
    fallbacks
        .filter(|x| x.vk == vk)
        .find(|x| modifiers_match(x.mods))
}

/// 指定の修飾キーがすべて押され、指定外の修飾キーが押されていないこと。
/// `Win+W` に対して `Win+Shift+W` で誤爆させないため、一致は厳密に見る。
fn modifiers_match(mods: HOT_KEY_MODIFIERS) -> bool {
    let down = |vk: VIRTUAL_KEY| (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0;
    let want = |m: HOT_KEY_MODIFIERS| (mods & m).0 != 0;

    (down(VK_LWIN) || down(VK_RWIN)) == want(MOD_WIN)
        && down(VK_CONTROL) == want(MOD_CONTROL)
        && down(VK_MENU) == want(MOD_ALT)
        && down(VK_SHIFT) == want(MOD_SHIFT)
}

/// Win 修飾の打鍵を握り潰すと、シェルが「Win 単独押下」とみなして
/// スタートメニューを開いてしまう。無害な Ctrl の打鍵を挟んで連鎖を切る。
fn break_win_chord() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CONTROL,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OWN_INPUT_TAG,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

/// `RegisterHotKey` で取れた場合と同じメッセージを流し、受け手を一本化する。
fn post_hotkey(id: i32) {
    let target = TARGET.with(|t| t.get());
    if target == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            Some(HWND(target as *mut _)),
            WM_HOTKEY,
            WPARAM(id as usize),
            LPARAM(0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Fallback, find_matching};
    use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_WIN};

    /// 同じ vk (Space) をメインホットキー (Win+Space) とクイックランチの
    /// ホットキー (Alt+Space) の両方がフックへ退避した場合、押されている
    /// 修飾キーに応じて正しい方を選べること。最初に登録された Fallback の
    /// vk だけを見て決め打ちすると、後から一致するはずの Fallback を
    /// 取りこぼす (修正前の実際のバグ)。
    #[test]
    fn picks_the_fallback_whose_modifiers_actually_match() {
        let fallbacks = [
            Fallback {
                id: 1,
                mods: MOD_WIN,
                vk: b' ' as u32,
            },
            Fallback {
                id: 2,
                mods: MOD_ALT,
                vk: b' ' as u32,
            },
        ];

        let held = MOD_ALT;
        let matched = find_matching(fallbacks.iter().copied(), b' ' as u32, |mods| mods == held);
        assert_eq!(matched.map(|f| f.id), Some(2));
    }

    #[test]
    fn returns_none_when_no_modifiers_match() {
        let fallbacks = [Fallback {
            id: 1,
            mods: MOD_WIN,
            vk: b' ' as u32,
        }];
        let matched = find_matching(fallbacks.iter().copied(), b' ' as u32, |_| false);
        assert!(matched.is_none());
    }

    #[test]
    fn ignores_fallbacks_with_a_different_vk() {
        let fallbacks = [Fallback {
            id: 1,
            mods: HOT_KEY_MODIFIERS(0),
            vk: b'W' as u32,
        }];
        let matched = find_matching(fallbacks.iter().copied(), b' ' as u32, |_| true);
        assert!(matched.is_none());
    }
}
