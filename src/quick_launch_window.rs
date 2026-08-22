//! 標準 Win32 コントロールだけで構成する Quick Launch 画面。

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FW_NORMAL, FW_SEMIBOLD,
    FillRect, GetMonitorInfoW, HBITMAP, HBRUSH, HDC, HFONT, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_SOLID, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_GETSEL, EM_REPLACESEL, EM_SETMARGINS, EM_SETSEL, ODS_SELECTED,
    SetWindowTheme,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, EN_CHANGE, GetClientRect, GetWindowTextLengthW,
    GetWindowTextW, HMENU, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL,
    LB_SETITEMHEIGHT, LBN_DBLCLK, LBS_HASSTRINGS, LBS_NOTIFY, LBS_OWNERDRAWFIXED, MoveWindow,
    PostMessageW, RegisterClassW, SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowTextW,
    ShowWindow, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_DRAWITEM, WM_ERASEBKGND, WM_KEYDOWN, WM_PAINT, WM_SETFONT, WM_SIZE,
    WM_SYSKEYDOWN, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP,
    WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::config::{Config, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch::{Action, Entry, Index};

const EDIT_ID: isize = 1001;
const LIST_ID: isize = 1002;
const WINDOW_WIDTH: i32 = 720;
const PADDING: i32 = 10;
const EDIT_HEIGHT: i32 = 34;
const ROW_HEIGHT: i32 = 42;
/// モードバッジ ("BOOKMARKS" 等) 用に検索窓の右側へ確保する幅。
const BADGE_WIDTH: i32 = 92;
/// 候補行のアイコン一辺。行の左端からの余白と種別バッジの半径もこれを基準に決める。
const ICON_SIZE: i32 = 26;
/// アイコンの左端 (行の左端からの距離)。
const ICON_LEFT: i32 = 8;
/// アイコンからテキストまでの隙間。
const ICON_TEXT_GAP: i32 = 10;
/// テキストの開始位置 (行の左端からの距離)。
const TEXT_LEFT: i32 = ICON_LEFT + ICON_SIZE + ICON_TEXT_GAP;

const BACKGROUND: COLORREF = rgb(13, 13, 13);
const SURFACE: COLORREF = rgb(30, 30, 30);
const SURFACE_HOVER: COLORREF = rgb(42, 42, 42);
const ACCENT: COLORREF = rgb(0, 120, 212);
const TEXT_PRIMARY: COLORREF = rgb(245, 245, 245);
const TEXT_SECONDARY: COLORREF = rgb(166, 166, 166);

/// モードバッジの背景色。プレフィックスごとに見分けが付くよう変える。
fn badge_color(badge: &str) -> COLORREF {
    match badge {
        "BOOKMARKS" => rgb(191, 90, 242), // 紫
        "WINDOWS" => rgb(48, 176, 199),   // シアン
        "APPS" => rgb(255, 159, 10),      // オレンジ
        "FILES" => rgb(52, 199, 89),      // 緑
        _ => ACCENT,
    }
}

/// 候補のアクション種別を表す色。バッジと同じ配色を流用し、
/// モードで絞り込む前 (通常検索の混在リスト) でも種別が一目で分かるようにする。
fn action_color(action: &Action) -> COLORREF {
    match action {
        Action::OpenFolder(_) => ACCENT,                  // 青 (フォルダ)
        Action::FocusWindow(_) => badge_color("WINDOWS"), // シアン
        Action::OpenUrl(_) => badge_color("BOOKMARKS"),   // 紫
        Action::LaunchApp => badge_color("APPS"),         // オレンジ
        Action::OpenWithDefaultHandler => badge_color("FILES"), // 緑 (Everything のファイル)
    }
}

pub const WM_QUICK_LAUNCH_EXECUTE: u32 = WM_APP + 4;
/// Everything からの検索結果を識別する `WM_COPYDATA` の `dwData` の初期値。
/// クエリごとに増やし、入力前の古い応答を判別できるようにする。
const EVERYTHING_REPLY_ID_START: u32 = WM_APP + 5;
/// `Ctrl+Shift+Enter` で選択項目を config へ登録するよう常駐部へ依頼する。
pub const WM_QUICK_LAUNCH_ADD_TO_FAVORITES: u32 = WM_APP + 6;
/// Quick Launch が一度に Everything へ要求する最大件数。
/// 全件表示はしない (`visible_results` の上限と同じ枠で足りる)。
const EVERYTHING_MAX_RESULTS: u32 = 24;
const CLASS_NAME: PCWSTR = w!("WaypointQuickLaunchWindow");

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Default)]
struct State {
    window: Option<HWND>,
    edit: Option<HWND>,
    list: Option<HWND>,
    owner: Option<HWND>,
    origin: Option<HWND>,
    index: Index,
    results: Vec<Entry>,
    pending: Option<Entry>,
    /// `Ctrl+Shift+Enter` で config への登録を要求された項目。
    /// ウィンドウは閉じずに続けて検索できるようにするため、
    /// `pending` (Enter で実行する項目) とは別に持つ。
    pending_add: Option<Entry>,
    /// `Ctrl+C` でパスをクリップボードへコピーした直後に立てる。
    /// 検索窓のバッジを一時的に `COPIED` へ差し替えるのに使い、
    /// 次のキー入力 (`update_results`) で通常のバッジへ戻る。
    copy_feedback: bool,
    visible_results: usize,
    dpi: u32,
    edit_font: Option<HFONT>,
    name_font: Option<HFONT>,
    detail_font: Option<HFONT>,
    background_brush: Option<HBRUSH>,
    surface_brush: Option<HBRUSH>,
    everything_enabled: bool,
    /// `f ` プレフィックスの間だけ立てる。プレフィックスを抜けた後に
    /// 遅れて届く Everything の応答を、無関係な検索結果へ混ぜないための
    /// ガード。
    everything_active: bool,
    /// 最後に送った Everything クエリの応答 ID。高速に入力したとき、
    /// 先行クエリの応答が後から届いて現在の候補を上書きするのを防ぐ。
    everything_reply_id: u32,
    /// `f ` モード中に有効な Everything 検索フラグ
    /// (`everything::MATCH_CASE` 等の OR 合成)。モードを抜けても値は保持し、
    /// 次に `f ` へ入ったときも同じ絞り込みを引き継ぐ。
    everything_flags: u32,
    /// 現在の入力が `b `/`w `/`a `/`f ` のいずれかに入っていれば
    /// そのモード名。検索窓のバッジ表示に使う。
    badge: Option<&'static str>,
}

pub fn configure(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        // インデックスの差し替えだけ借用内で行い、描画は借用を解放してから
        let has_window = {
            let mut state = state.borrow_mut();
            state.index = Index::build(config, dynamic);
            state.visible_results = config.settings.quick_launch.visible_results.clamp(12, 24);
            state.everything_enabled = config.settings.quick_launch.include_everything;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

pub fn show(owner: HWND, origin: Option<HWND>) -> Result<()> {
    ensure_window(owner)?;
    let (window, edit, visible_results) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.owner = Some(owner);
        state.origin = origin;
        (state.window, state.edit, state.visible_results)
    });
    let (Some(window), Some(edit)) = (window, edit) else {
        return Ok(());
    };
    unsafe {
        // SetWindowTextW は EN_CHANGE を同期送信するため、STATE の借用外で呼ぶ。
        let _ = SetWindowTextW(edit, w!(""));
    }
    let monitor_window = origin.unwrap_or(owner);
    let dpi = unsafe { GetDpiForWindow(monitor_window) }.max(96);
    apply_dpi(window, dpi);
    position_window(window, monitor_window, visible_results, dpi);
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        let _ = SetFocus(Some(edit));
    }
    Ok(())
}

pub fn take_pending() -> Option<(Entry, Option<HWND>)> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let origin = state.origin;
        state.pending.take().map(|entry| (entry, origin))
    })
}

pub fn take_pending_add() -> Option<Entry> {
    STATE.with(|state| state.borrow_mut().pending_add.take())
}

/// Quick Launch の子コントロール宛てキーを通常の DispatchMessage より先に扱う。
pub fn handle_message(message: &windows::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    let belongs_to_quick_launch = STATE.with(|state| {
        let state = state.borrow();
        Some(message.hwnd) == state.edit || Some(message.hwnd) == state.list
    });
    if !belongs_to_quick_launch {
        return false;
    }
    // Alt 併用のキーは WM_KEYDOWN ではなく WM_SYSKEYDOWN で届く。
    // f モードのフィルタ切替 (Alt+C/W/R) だけここで拾い、素通しすると
    // Windows がメニューアクセラレータとして扱いビープ音を鳴らすのを防ぐ。
    if message.message == WM_SYSKEYDOWN {
        return match message.wParam.0 as u32 {
            0x43 => toggle_everything_flag(crate::everything::MATCH_CASE), // Alt+C
            0x57 => toggle_everything_flag(crate::everything::MATCH_WHOLE_WORD), // Alt+W
            0x52 => toggle_everything_flag(crate::everything::REGEX),      // Alt+R
            _ => false,
        };
    }
    if message.message != WM_KEYDOWN {
        return false;
    }
    match message.wParam.0 as u32 {
        0x1b => hide_window(STATE.with(|state| state.borrow().window)), // Esc
        0x26 => move_selection(-1),
        0x28 => move_selection(1),
        0x24 => select_at(STATE.with(|state| state.borrow().list), 0),
        0x23 => {
            let (list, count) = STATE.with(|state| {
                let state = state.borrow();
                (state.list, state.results.len())
            });
            select_at(list, count.saturating_sub(1));
        }
        0x21 => move_selection(-10),
        0x22 => move_selection(10),
        0x0d if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0
            && unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 =>
        {
            add_selected_to_favorites();
        }
        0x0d => queue_selected(),
        // Ctrl+C: 選択中候補のパスをクリップボードへコピーする。
        // Edit にフォーカスがある間は通常のテキストコピーを奪わないよう、
        // リスト側で押されたときだけ扱う。
        0x43 if Some(message.hwnd) == STATE.with(|state| state.borrow().list)
            && unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 =>
        {
            copy_selected_path();
        }
        // Ctrl+E: 選択中候補をエクスプローラーで開き、対象を選択状態にする。
        0x45 if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 => {
            reveal_selected_in_explorer();
        }
        0x08 if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 => {
            let edit = STATE.with(|state| state.borrow().edit);
            let Some(edit) = edit else {
                return false;
            };
            delete_word_before_cursor(edit);
        }
        _ => return false,
    }
    true
}

/// `f ` モード中だけ有効な Everything 検索フラグの切替。
/// モード外では何もしない (Alt+C 等を他モードで押しても無反応)。
/// 切替後は同じクエリを新しいフラグで即座に再送する。
fn toggle_everything_flag(flag: u32) -> bool {
    let in_everything_mode = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.everything_active {
            return false;
        }
        state.everything_flags ^= flag;
        true
    });
    if !in_everything_mode {
        return false;
    }
    STATE.with(|state| {
        let (window, dpi) = {
            let state = state.borrow();
            (state.window, state.dpi)
        };
        invalidate_search_bar(window, dpi);
        update_results(state);
    });
    true
}

fn ensure_window(owner: HWND) -> Result<()> {
    if STATE.with(|state| state.borrow().window.is_some()) {
        return Ok(());
    }
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let background_brush = CreateSolidBrush(BACKGROUND);
        let surface_brush = CreateSolidBrush(SURFACE);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            hbrBackground: background_brush,
            ..Default::default()
        };
        let _ = RegisterClassW(&class);

        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS_NAME,
            w!("Quick Launch - Waypoint"),
            WS_POPUP,
            0,
            0,
            WINDOW_WIDTH,
            360,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )?;
        let edit = CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            PADDING,
            PADDING,
            WINDOW_WIDTH - PADDING * 2,
            EDIT_HEIGHT,
            Some(window),
            Some(HMENU(EDIT_ID as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let list = CreateWindowExW(
            Default::default(),
            w!("LISTBOX"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS) as u32),
            PADDING,
            PADDING + EDIT_HEIGHT + PADDING,
            WINDOW_WIDTH - PADDING * 2,
            260,
            Some(window),
            Some(HMENU(LIST_ID as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let _ = SetWindowTheme(edit, w!("DarkMode_Explorer"), PCWSTR::null());
        let _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null());
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.window = Some(window);
            state.edit = Some(edit);
            state.list = Some(list);
            state.owner = Some(owner);
            state.background_brush = Some(background_brush);
            state.surface_brush = Some(surface_brush);
        });
        apply_dpi(window, GetDpiForWindow(window));
        apply_window_chrome(window);
    }
    Ok(())
}

/// Win32 から呼ばれる入口。
///
/// `extern "system"` は unwind できないため、中で panic すると
/// 「panic in a function that cannot unwind」で即 abort する
/// (GUI サブシステムでは stderr も出ないので無言で消える)。
/// 実際に RefCell の再入借用でこれを踏んだ。原因を潰したうえで、
/// 再発時に落ちないよう捕まえてログに残す。
extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let handled = std::panic::catch_unwind(|| dispatch(hwnd, message, wparam, lparam));
    match handled {
        Ok(result) => result,
        Err(_) => {
            // panic フックが既に詳細を記録している。ここでは
            // どのメッセージで落ちたかを補足する
            crate::panic_log::record(&format!(
                "quick launch window_proc panicked on message 0x{message:04x}; recovered"
            ));
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            let control = HWND(lparam.0 as *mut _);
            let is_edit = STATE.with(|state| Some(control) == state.borrow().edit);
            let is_list = STATE.with(|state| Some(control) == state.borrow().list);
            if is_edit && notification == EN_CHANGE {
                STATE.with(update_results);
            } else if is_list && notification == LBN_DBLCLK {
                queue_selected();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            // MoveWindow は WM_ERASEBKGND / WM_PAINT を同期送信して
            // window_proc を再入させる。借用を解放してから呼ぶ
            let (edit, list, dpi) = STATE.with(|state| {
                let state = state.borrow();
                (state.edit, state.list, state.dpi)
            });
            let padding = scale(PADDING, dpi);
            let edit_height = scale(EDIT_HEIGHT, dpi);
            let badge_width = scale(BADGE_WIDTH, dpi);
            unsafe {
                if let Some(edit) = edit {
                    let _ = MoveWindow(
                        edit,
                        padding + scale(6, dpi),
                        padding + scale(6, dpi),
                        width - padding * 2 - badge_width - scale(14, dpi),
                        edit_height - scale(12, dpi),
                        true,
                    );
                }
                if let Some(list) = list {
                    let top = padding + edit_height + scale(6, dpi);
                    let _ = MoveWindow(
                        list,
                        padding,
                        top,
                        width - padding * 2,
                        height - top - padding,
                        true,
                    );
                }
            }
            LRESULT(0)
        }
        WM_DRAWITEM => {
            if lparam.0 != 0 {
                unsafe { draw_list_item(&*(lparam.0 as *const DRAWITEMSTRUCT)) };
            }
            LRESULT(1)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as *mut _);
            STATE.with(|state| {
                let state = state.borrow();
                unsafe {
                    SetTextColor(hdc, TEXT_PRIMARY);
                    SetBkColor(hdc, SURFACE);
                }
                LRESULT(state.surface_brush.map_or(0, |brush| brush.0 as isize))
            })
        }
        WM_CTLCOLORLISTBOX => {
            let hdc = HDC(wparam.0 as *mut _);
            STATE.with(|state| {
                let state = state.borrow();
                unsafe {
                    SetTextColor(hdc, TEXT_PRIMARY);
                    SetBkColor(hdc, BACKGROUND);
                }
                LRESULT(state.background_brush.map_or(0, |brush| brush.0 as isize))
            })
        }
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut _);
            let mut rect = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut rect);
            }
            STATE.with(|state| {
                if let Some(brush) = state.borrow().background_brush {
                    unsafe {
                        FillRect(hdc, &rect, brush);
                    }
                }
            });
            LRESULT(1)
        }
        WM_PAINT => {
            paint_window(hwnd);
            LRESULT(0)
        }
        WM_ACTIVATE if (wparam.0 & 0xffff) == 0 => {
            hide_window(Some(hwnd));
            LRESULT(0)
        }
        WM_CLOSE => {
            hide_window(Some(hwnd));
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA => {
            if lparam.0 != 0 {
                unsafe {
                    let copy_data =
                        &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                    let reply_id = copy_data.dwData as u32;
                    let is_current = STATE.with(|state| {
                        let state = state.borrow();
                        accepts_everything_reply(
                            state.everything_active,
                            state.everything_reply_id,
                            reply_id,
                        )
                    });
                    if is_current && !copy_data.lpData.is_null() && copy_data.cbData > 0 {
                        // Everything はこのハンドラから戻ると lpData を解放する。
                        // 保持するならここでコピーする必要がある (SDK の注記通り)
                        let bytes = std::slice::from_raw_parts(
                            copy_data.lpData.cast::<u8>(),
                            copy_data.cbData as usize,
                        )
                        .to_vec();
                        handle_everything_results(reply_id, &bytes);
                    }
                }
            }
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// 検索結果を作り直し、リストボックスへ反映する。
///
/// `SendMessageW` は同期呼び出しで、リストボックスは所有者へ
/// `WM_ERASEBKGND` / `WM_DRAWITEM` をその場で送り返す。再入した
/// `window_proc` は STATE を `borrow()` するため、**借用を保持したまま
/// Win32 を呼んではいけない** (保持すると BorrowMutError で panic し、
/// `window_proc` は unwind 不可なので abort する)。
/// 借用中は検索と `results` の更新だけを行い、描画用の値を取り出してから
/// 借用を解放し、その後で `SendMessageW` を呼ぶ。
/// 検索窓に出すモードバッジを入力文字列から判定し、変わっていれば
/// 検索窓部分だけ再描画する。
fn update_badge(state: &RefCell<State>, query: &str) {
    let badge = crate::quick_launch::prefix_badge(query);
    let (window, dpi, changed) = {
        let mut state = state.borrow_mut();
        let changed = state.badge != badge;
        state.badge = badge;
        (state.window, state.dpi, changed)
    };
    if changed {
        invalidate_search_bar(window, dpi);
    }
}

/// 検索窓 (バッジを含む上部の帯) だけを再描画対象にする。
/// リスト部分を巻き込まないことで、バッジ更新のたびにリスト全体が
/// ちらつくのを防ぐ。
fn invalidate_search_bar(window: Option<HWND>, dpi: u32) {
    let Some(window) = window else {
        return;
    };
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        let search_rect = RECT {
            left: 0,
            top: 0,
            right: client.right,
            bottom: scale(PADDING, dpi) * 2 + scale(EDIT_HEIGHT, dpi),
        };
        let _ = InvalidateRect(Some(window), Some(&search_rect), false);
    }
}

fn update_results(state: &RefCell<State>) {
    // read_text も Win32 呼び出しなので借用の外で済ませる
    let edit = state.borrow().edit;
    let query = edit.map(read_text).unwrap_or_default();
    update_badge(state, &query);

    if let Some(rest) = query.strip_prefix(crate::quick_launch::EVERYTHING_PREFIX) {
        start_everything_query(state, rest);
        return;
    }

    let (list, labels, has_results) = {
        let mut state = state.borrow_mut();
        // プレフィックスを外れたら、遅れて届く Everything の応答を無視させる
        state.everything_active = false;
        state.copy_feedback = false;
        state.results = state.index.search(&query).into_iter().cloned().collect();
        let labels: Vec<HSTRING> = state
            .results
            .iter()
            .map(|entry| HSTRING::from(format!("{}    {}", entry.name, entry_context(entry))))
            .collect();
        (state.list, labels, !state.results.is_empty())
    }; // ← ここで借用が切れる。以降の再入は borrow() できる

    let Some(list) = list else {
        return;
    };
    populate_list(list, &labels, has_results);
}

/// `f ` プレフィックスに入った。Everything へ非同期クエリを送り、
/// 結果が届くまでの間はリストを空にする。
///
/// Everything 未起動・設定で無効の場合は何も送らず空のまま。
/// 空の検索語 (`f ` だけ) はクエリを送らない — 全件検索は重く、
/// タイプの途中で毎回投げると Everything 側の応答待ちが積み上がる。
fn start_everything_query(state: &RefCell<State>, text: &str) {
    let (window, list, enabled, flags, reply_id) = {
        let mut state = state.borrow_mut();
        state.everything_active = true;
        state.results.clear();
        state.everything_reply_id = next_everything_reply_id(state.everything_reply_id);
        (
            state.window,
            state.list,
            state.everything_enabled,
            state.everything_flags,
            state.everything_reply_id,
        )
    };
    if let Some(list) = list {
        populate_list(list, &[], false);
    }
    let (Some(window), true, false) = (window, enabled, text.is_empty()) else {
        return;
    };
    crate::everything::query(window, reply_id, text, EVERYTHING_MAX_RESULTS, flags);
}

/// Everything から届いた `WM_COPYDATA` を結果リストへ反映する。
///
/// `f ` を抜けていれば `update_results` が `everything_active` を
/// 下ろしているので、届いた結果はここでは扱わず捨てる (プレフィックスが
/// 外れた後に遅延到着した応答が、無関係な検索結果へ紛れ込むのを防ぐ)。
fn handle_everything_results(reply_id: u32, data: &[u8]) {
    let parsed = crate::everything::parse_results(data);
    let outcome = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if !accepts_everything_reply(state.everything_active, state.everything_reply_id, reply_id) {
            return None;
        }
        state.results = parsed
            .into_iter()
            .map(|item| crate::quick_launch::Entry {
                name: item.name,
                breadcrumb: String::new(),
                path: item.path,
                action: if item.is_folder {
                    crate::quick_launch::Action::OpenFolder(OpenMode::NewWindow)
                } else {
                    crate::quick_launch::Action::OpenWithDefaultHandler
                },
                branch: None,
            })
            .collect();
        let labels: Vec<HSTRING> = state
            .results
            .iter()
            .map(|entry| HSTRING::from(format!("{}    {}", entry.name, entry_context(entry))))
            .collect();
        Some((state.list, labels, !state.results.is_empty()))
    });
    let Some((list, labels, has_results)) = outcome else {
        return;
    };
    if let Some(list) = list {
        populate_list(list, &labels, has_results);
    }
}

fn next_everything_reply_id(current: u32) -> u32 {
    current
        .checked_add(1)
        .filter(|next| *next >= EVERYTHING_REPLY_ID_START)
        .unwrap_or(EVERYTHING_REPLY_ID_START)
}

fn accepts_everything_reply(active: bool, expected: u32, received: u32) -> bool {
    active && expected == received
}

/// リストボックスの中身を丸ごと差し替える。通常検索と Everything の
/// 非同期結果受信 (`handle_everything_results`) の双方から使う。
fn populate_list(list: HWND, labels: &[HSTRING], has_results: bool) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_RESETCONTENT,
            None,
            None,
        );
        for label in labels {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        if has_results {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_SETCURSEL,
                Some(WPARAM(0)),
                None,
            );
        }
    }
}

fn read_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        let mut text = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut text);
        String::from_utf16_lossy(&text[..copied as usize])
    }
}

/// カーソル直前の単語の開始位置を返す。
/// 「単語」は空白の連続とそれ以外の連続の境界で区切る単純な定義。
fn word_start_before(text: &[u16], cursor: usize) -> usize {
    let mut start = cursor;
    while start > 0 && text[start - 1] == b' ' as u16 {
        start -= 1;
    }
    while start > 0 && text[start - 1] != b' ' as u16 {
        start -= 1;
    }
    start
}

/// Ctrl+Backspace: カーソル直前の単語を削除する。
fn delete_word_before_cursor(edit: HWND) {
    let text: Vec<u16> = read_text(edit).encode_utf16().collect();
    let mut sel_start = 0u32;
    let mut sel_end = 0u32;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_GETSEL,
            Some(WPARAM(&mut sel_start as *mut u32 as usize)),
            Some(LPARAM(&mut sel_end as *mut u32 as isize)),
        );
    }
    let cursor = sel_start.min(sel_end) as usize;
    if cursor == 0 {
        return;
    }
    let start = word_start_before(&text, cursor);
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(start)),
            Some(LPARAM(cursor as isize)),
        );
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(w!("").as_ptr() as isize)),
        );
    }
}

/// リストボックスの選択位置を読む。
///
/// `SendMessageW` を呼ぶため、STATE を借用したまま渡さないこと。
/// 引数はハンドルだけを受け取る。
fn current_selection(list: Option<HWND>) -> Option<usize> {
    let list = list?;
    let selected = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(list, LB_GETCURSEL, None, None).0
    };
    (selected >= 0).then_some(selected as usize)
}

fn select_at(list: Option<HWND>, index: usize) {
    let Some(list) = list else {
        return;
    };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_SETCURSEL,
            Some(WPARAM(index)),
            None,
        );
    }
}

/// 選択を相対移動する。借用は最初に済ませ、以降は Win32 のみ触る。
fn move_selection(delta: isize) {
    let (list, count) = STATE.with(|state| {
        let state = state.borrow();
        (state.list, state.results.len())
    });
    if count == 0 {
        return;
    }
    let current = current_selection(list).unwrap_or(0);
    let next = current.saturating_add_signed(delta).min(count - 1);
    select_at(list, next);
}

fn queue_selected() {
    // LB_GETCURSEL は SendMessageW。借用の外で読んでおく
    let list = STATE.with(|state| state.borrow().list);
    let selected = current_selection(list);

    let queued = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut entry = selected
            .and_then(|index| state.results.get(index))
            .cloned()?;
        if let Action::OpenFolder(mode) = &mut entry.action {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            if shift {
                *mode = OpenMode::NewWindow;
            } else if control {
                *mode = OpenMode::Reuse;
            }
        }
        state.pending = Some(entry);
        Some((state.window, state.owner))
    });
    let Some((window, owner)) = queued else {
        return;
    };
    hide_window(window);
    if let Some(owner) = owner {
        unsafe {
            let _ = PostMessageW(Some(owner), WM_QUICK_LAUNCH_EXECUTE, WPARAM(0), LPARAM(0));
        }
    }
}

/// 現在選択中の候補を複製して返す。`SendMessageW` を挟むため、
/// 呼び出し側は返り値を得てから STATE を再度借用すること。
fn selected_entry() -> Option<Entry> {
    let list = STATE.with(|state| state.borrow().list);
    let selected = current_selection(list)?;
    STATE.with(|state| state.borrow().results.get(selected).cloned())
}

/// `Ctrl+Shift+Enter`: 選択中の候補を config へ登録するよう常駐部へ
/// 依頼する。ウィンドウは閉じず、続けて他の候補も登録できるようにする。
fn add_selected_to_favorites() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.to_item().is_none() {
        // ウィンドウ / URL 候補は登録対象外。無音で無視する
        // (FocusWindow・OpenUrl は config へ永続化できないため)
        return;
    }
    let owner = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.pending_add = Some(entry);
        state.owner
    });
    if let Some(owner) = owner {
        unsafe {
            let _ = PostMessageW(
                Some(owner),
                WM_QUICK_LAUNCH_ADD_TO_FAVORITES,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// `Ctrl+C`: 選択中候補のパスをクリップボードへコピーする。
fn copy_selected_path() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.path.is_empty() || crate::clipboard::set_text(&entry.path).is_err() {
        return;
    }
    STATE.with(|state| {
        let (window, dpi) = {
            let mut state = state.borrow_mut();
            state.copy_feedback = true;
            (state.window, state.dpi)
        };
        invalidate_search_bar(window, dpi);
    });
}

/// `Ctrl+E`: 選択中候補をエクスプローラーで開き、対象を選択状態にする。
fn reveal_selected_in_explorer() {
    let Some(entry) = selected_entry() else {
        return;
    };
    if entry.path.is_empty() {
        return;
    }
    let window = STATE.with(|state| state.borrow().window);
    hide_window(window);
    let _ = crate::shell::reveal_in_explorer(&entry.path);
}

fn hide_window(window: Option<HWND>) {
    if let Some(window) = window {
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
    }
}

fn position_window(window: HWND, monitor_window: HWND, rows: usize, dpi: u32) {
    unsafe {
        let monitor = MonitorFromWindow(monitor_window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut work = RECT::default();
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            work = info.rcWork;
        } else {
            let _ = GetClientRect(window, &mut work);
        }
        let width = scale(WINDOW_WIDTH, dpi);
        let height = scale(
            PADDING * 3 + EDIT_HEIGHT + 6 + ROW_HEIGHT * rows as i32 + 36,
            dpi,
        );
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = MoveWindow(window, x, y, width, height, true);
    }
}

fn apply_dpi(window: HWND, dpi: u32) {
    let dpi = dpi.max(96);
    let (edit, list, old_fonts, fonts) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.dpi == dpi && state.edit_font.is_some() {
            return (
                state.edit,
                state.list,
                Vec::new(),
                (state.edit_font, state.name_font, state.detail_font),
            );
        }
        let old_fonts = [state.edit_font, state.name_font, state.detail_font]
            .into_iter()
            .flatten()
            .collect();
        state.dpi = dpi;
        state.edit_font = create_font(scale(14, dpi), FW_NORMAL.0 as i32);
        state.name_font = create_font(scale(14, dpi), FW_SEMIBOLD.0 as i32);
        state.detail_font = create_font(scale(11, dpi), FW_NORMAL.0 as i32);
        (
            state.edit,
            state.list,
            old_fonts,
            (state.edit_font, state.name_font, state.detail_font),
        )
    });

    unsafe {
        if let (Some(edit), Some(font)) = (edit, fonts.0) {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let left = scale(2, dpi) as u32;
            let right = scale(8, dpi) as u32;
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                EM_SETMARGINS,
                Some(WPARAM(3)),
                Some(LPARAM(((right << 16) | left) as isize)),
            );
        }
        if let Some(list) = list {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_SETITEMHEIGHT,
                Some(WPARAM(0)),
                Some(LPARAM(scale(ROW_HEIGHT, dpi) as isize)),
            );
        }
        for font in old_fonts {
            let _ = DeleteObject(font.into());
        }
        let _ = InvalidateRect(Some(window), None, true);
    }
}

fn create_font(pixel_height: i32, weight: i32) -> Option<HFONT> {
    let font = unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0.into(),
            w!("Segoe UI Variable Text"),
        )
    };
    (!font.is_invalid()).then_some(font)
}

fn apply_window_chrome(window: HWND) {
    unsafe {
        let dark = 1i32;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&corner).cast(),
            size_of_val(&corner) as u32,
        );
    }
}

fn paint_window(window: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(window, &mut paint);
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        let (dpi, background, surface, badge, detail_font, everything_flags, everything_active) =
            STATE.with(|state| {
                let state = state.borrow();
                let badge = if state.copy_feedback {
                    Some("COPIED")
                } else {
                    state.badge
                };
                (
                    state.dpi,
                    state.background_brush,
                    state.surface_brush,
                    badge,
                    state.detail_font,
                    state.everything_flags,
                    state.everything_active,
                )
            });
        if let Some(background) = background {
            FillRect(hdc, &client, background);
        }
        if let Some(surface) = surface {
            let padding = scale(PADDING, dpi);
            let edit_height = scale(EDIT_HEIGHT, dpi);
            let search = RECT {
                left: padding,
                top: padding,
                right: client.right - padding,
                bottom: padding + edit_height,
            };
            let surface_pen = CreatePen(PS_SOLID, 1, SURFACE);
            let old_pen = SelectObject(hdc, surface_pen.into());
            let old_brush = SelectObject(hdc, surface.into());
            let radius = scale(10, dpi);
            let _ = RoundRect(
                hdc,
                search.left,
                search.top,
                search.right,
                search.bottom,
                radius,
                radius,
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(surface_pen.into());

            if let Some(badge) = badge {
                draw_badge(hdc, badge, search, dpi, detail_font);
                if everything_active {
                    draw_everything_flag_badges(hdc, everything_flags, search, dpi, detail_font);
                }
            } else {
                draw_clock(hdc, search, dpi, detail_font);
            }
        }
        let _ = EndPaint(window, &paint);
    }
}

/// 検索窓の右端に、アクティブなプレフィックスモードの名前を丸バッジで描く。
unsafe fn draw_badge(hdc: HDC, badge: &str, search: RECT, dpi: u32, detail_font: Option<HFONT>) {
    unsafe {
        let color = badge_color(badge);
        let height = scale(20, dpi);
        let width = scale(BADGE_WIDTH, dpi) - scale(16, dpi);
        let rect = RECT {
            left: search.right - scale(10, dpi) - width,
            top: search.top + (search.bottom - search.top - height) / 2,
            right: search.right - scale(10, dpi),
            bottom: search.top + (search.bottom - search.top - height) / 2 + height,
        };
        let brush = CreateSolidBrush(color);
        let radius = height / 2;
        let old_brush = SelectObject(hdc, brush.into());
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_pen = SelectObject(hdc, pen.into());
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());

        if let Some(font) = detail_font {
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, TEXT_PRIMARY);
            let mut text_rect = rect;
            draw_text_centered(hdc, badge, &mut text_rect);
            SelectObject(hdc, old_font);
        }
    }
}

/// バッジ非表示中の検索窓の右端に、現在時刻 (HH:mm) を淡色で描く。
unsafe fn draw_clock(hdc: HDC, search: RECT, dpi: u32, detail_font: Option<HFONT>) {
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let text = format!("{:02}:{:02}", time.wHour, time.wMinute);
    unsafe {
        let Some(font) = detail_font else { return };
        let width = scale(BADGE_WIDTH, dpi) - scale(16, dpi);
        let mut rect = RECT {
            left: search.right - scale(10, dpi) - width,
            top: search.top,
            right: search.right - scale(10, dpi),
            bottom: search.bottom,
        };
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, TEXT_SECONDARY);
        draw_text_centered(hdc, &text, &mut rect);
        SelectObject(hdc, old_font);
    }
}

/// `f ` モードでアクティブな Everything 検索フラグを、モードバッジの
/// 左側へ小さいピルとして並べて描く (Alt+C/W/R でトグルした状態の可視化)。
unsafe fn draw_everything_flag_badges(
    hdc: HDC,
    flags: u32,
    search: RECT,
    dpi: u32,
    detail_font: Option<HFONT>,
) {
    let active: Vec<&str> = [
        (crate::everything::MATCH_CASE, "Cc"),
        (crate::everything::MATCH_WHOLE_WORD, "W"),
        (crate::everything::REGEX, ".*"),
    ]
    .into_iter()
    .filter(|(flag, _)| flags & flag != 0)
    .map(|(_, label)| label)
    .collect();
    if active.is_empty() {
        return;
    }

    unsafe {
        let height = scale(20, dpi);
        let gap = scale(6, dpi);
        let badge_left_edge =
            search.right - scale(10, dpi) - (scale(BADGE_WIDTH, dpi) - scale(16, dpi));
        let mut right = badge_left_edge - gap;
        for label in active {
            let width = scale(10, dpi) * 2 + scale(8, dpi) * label.chars().count() as i32;
            let rect = RECT {
                left: right - width,
                top: search.top + (search.bottom - search.top - height) / 2,
                right,
                bottom: search.top + (search.bottom - search.top - height) / 2 + height,
            };
            let brush = CreateSolidBrush(SURFACE_HOVER);
            let pen = CreatePen(PS_SOLID, 1, ACCENT);
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = height / 2;
            let _ = RoundRect(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                radius,
                radius,
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());

            if let Some(font) = detail_font {
                let old_font = SelectObject(hdc, font.into());
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, ACCENT);
                let mut text_rect = rect;
                draw_text_centered(hdc, label, &mut text_rect);
                SelectObject(hdc, old_font);
            }

            right = rect.left - gap;
        }
    }
}

unsafe fn draw_list_item(draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let Some((entry, name_font, detail_font, dpi, badge)) = STATE.with(|state| {
        let state = state.borrow();
        let entry = state.results.get(draw.itemID as usize)?.clone();
        Some((
            entry,
            state.name_font,
            state.detail_font,
            state.dpi,
            state.badge,
        ))
    }) else {
        return;
    };

    unsafe {
        let selected = draw.itemState.0 & ODS_SELECTED.0 != 0;
        let background = CreateSolidBrush(BACKGROUND);
        FillRect(draw.hDC, &draw.rcItem, background);
        let _ = DeleteObject(background.into());

        // 選択行はカード風に少し内側へ収め、角を丸めて他の行から浮かせる
        if selected {
            let accent_color = badge.map_or(ACCENT, badge_color);
            let inset = scale(2, dpi);
            let card = RECT {
                left: draw.rcItem.left + inset,
                top: draw.rcItem.top + scale(1, dpi),
                right: draw.rcItem.right - inset,
                bottom: draw.rcItem.bottom - scale(1, dpi),
            };
            let card_brush = CreateSolidBrush(SURFACE_HOVER);
            let card_pen = CreatePen(PS_SOLID, 1, SURFACE_HOVER);
            let old_brush = SelectObject(draw.hDC, card_brush.into());
            let old_pen = SelectObject(draw.hDC, card_pen.into());
            let radius = scale(8, dpi);
            let _ = RoundRect(
                draw.hDC,
                card.left,
                card.top,
                card.right,
                card.bottom,
                radius,
                radius,
            );
            SelectObject(draw.hDC, old_brush);
            SelectObject(draw.hDC, old_pen);
            let _ = DeleteObject(card_brush.into());
            let _ = DeleteObject(card_pen.into());

            let accent = CreateSolidBrush(accent_color);
            let accent_rect = RECT {
                left: card.left,
                top: card.top + scale(6, dpi),
                right: card.left + scale(3, dpi),
                bottom: card.bottom - scale(6, dpi),
            };
            FillRect(draw.hDC, &accent_rect, accent);
            let _ = DeleteObject(accent.into());
        }

        draw_icon_backdrop(draw.hDC, action_color(&entry.action), draw.rcItem, dpi);
        match entry.action {
            Action::OpenFolder(_) | Action::OpenWithDefaultHandler | Action::LaunchApp => {
                draw_path_icon(draw.hDC, &entry.path, draw.rcItem, dpi)
            }
            Action::FocusWindow(hwnd) => {
                draw_window_icon(draw.hDC, HWND(hwnd as *mut _), draw.rcItem, dpi)
            }
            Action::OpenUrl(_) => draw_favicon_icon(draw.hDC, &entry.path, draw.rcItem, dpi),
        }
        SetBkMode(draw.hDC, TRANSPARENT);
        let text_left = draw.rcItem.left + scale(TEXT_LEFT, dpi);
        let text_right = draw.rcItem.right - scale(8, dpi);

        if let Some(font) = name_font {
            let old = SelectObject(draw.hDC, font.into());
            SetTextColor(draw.hDC, TEXT_PRIMARY);
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(2, dpi),
                right: text_right,
                bottom: draw.rcItem.top + scale(23, dpi),
            };
            let label = crate::git::with_branch(&entry.name, entry.branch.as_deref());
            draw_text(draw.hDC, &label, &mut rect);
            SelectObject(draw.hDC, old);
        }

        if let Some(font) = detail_font {
            let old = SelectObject(draw.hDC, font.into());
            SetTextColor(draw.hDC, TEXT_SECONDARY);
            let detail = entry_context(&entry);
            let mut rect = RECT {
                left: text_left,
                top: draw.rcItem.top + scale(20, dpi),
                right: text_right,
                bottom: draw.rcItem.bottom - scale(1, dpi),
            };
            draw_text(draw.hDC, &detail, &mut rect);
            SelectObject(draw.hDC, old);
        }
    }
}

/// リスト・詳細行に出す補足テキスト。パスが無い (ウィンドウ項目) 場合は
/// breadcrumb だけを出す。
fn entry_context(entry: &Entry) -> String {
    if entry.path.is_empty() {
        entry.breadcrumb.clone()
    } else if entry.breadcrumb.is_empty() {
        entry.path.clone()
    } else {
        format!("{}  —  {}", entry.breadcrumb, entry.path)
    }
}

unsafe fn draw_text(hdc: HDC, text: &str, rect: &mut RECT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

/// バッジの文字を水平・垂直とも中央揃えで描く。
unsafe fn draw_text_centered(hdc: HDC, text: &str, rect: &mut RECT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        DrawTextW(
            hdc,
            &mut wide,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
        );
    }
}

/// 種別色の丸背景。実アイコンより一回り大きく敷き、透明な余白を持つ
/// アイコン (フォルダ・URL 等) でも種別が一目で分かるようにする。
unsafe fn draw_icon_backdrop(hdc: HDC, color: COLORREF, rect: RECT, dpi: u32) {
    unsafe {
        let size = scale(ICON_SIZE + 6, dpi);
        let left = rect.left + scale(ICON_LEFT, dpi) - scale(3, dpi);
        let top = rect.top + (rect.bottom - rect.top - size) / 2;
        let brush = CreateSolidBrush(backdrop_tint(color));
        let pen = CreatePen(PS_SOLID, 1, backdrop_tint(color));
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = Ellipse(hdc, left, top, left + size, top + size);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

/// 種別色をそのまま塗ると強すぎるので、背景 (`BACKGROUND`) に大きく
/// 寄せた低彩度版にする。
fn backdrop_tint(color: COLORREF) -> COLORREF {
    let mix = |channel: u8| -> u8 { ((channel as u32 * 46 + 13 * 210) / 256) as u8 };
    rgb(
        mix((color.0 & 0xff) as u8),
        mix(((color.0 >> 8) & 0xff) as u8),
        mix(((color.0 >> 16) & 0xff) as u8),
    )
}

unsafe fn draw_path_icon(hdc: HDC, path: &str, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    // shell:MyComputerFolder 等はファイルパスではないため専用の解決経路を使う
    let bitmap = if path.starts_with("shell:") {
        crate::icon::bitmap_for_shell_sized(path, size)
    } else {
        crate::icon::bitmap_for_sized(path, size)
    };
    let Some(bitmap) = bitmap else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

unsafe fn draw_window_icon(hdc: HDC, hwnd: HWND, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    let Some(bitmap) = crate::icon::bitmap_for_window_sized(hwnd, size) else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

/// favicon が無いブックマークに使う既定アイコン (星マーク)。
const ICON_BOOKMARK: &[u8] = include_bytes!("../assets/menu/bookmark.png");

/// ブックマークの favicon を描く。Chrome/Edge の `Favicons` DB に
/// 見つからなければ既定の星マークへフォールバックする。
unsafe fn draw_favicon_icon(hdc: HDC, url: &str, rect: RECT, dpi: u32) {
    let size = scale(ICON_SIZE, dpi);
    let bitmap = crate::icon::bitmap_for_favicon_sized(url, size)
        .or_else(|| crate::icon::bitmap_for_asset_sized("bookmark", ICON_BOOKMARK, size));
    let Some(bitmap) = bitmap else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

/// `bitmap` は既に `size` 四方で生成済みの前提で等倍コピーする。
///
/// 寸法違いのビットマップを AlphaBlend で拡大縮小すると輪郭がにじむため、
/// 呼び出し側 (`icon::bitmap_for_sized` / `bitmap_for_window_sized`) で
/// 要求寸法どおりのビットマップを取得させ、ここでは伸縮させない。
unsafe fn draw_icon_bitmap(hdc: HDC, bitmap: HBITMAP, rect: RECT, dpi: u32, size: i32) {
    unsafe {
        let source = CreateCompatibleDC(Some(hdc));
        if source.is_invalid() {
            return;
        }
        let old = SelectObject(source, bitmap.into());
        let _ = AlphaBlend(
            hdc,
            rect.left + scale(ICON_LEFT, dpi),
            rect.top + (rect.bottom - rect.top - size) / 2,
            size,
            size,
            source,
            0,
            0,
            size,
            size,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            },
        );
        SelectObject(source, old);
        let _ = DeleteDC(source);
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi.max(96) as i32 / 96
}

#[cfg(test)]
mod tests {
    use super::{accepts_everything_reply, next_everything_reply_id, word_start_before};

    fn to_utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn deletes_last_word() {
        let text = to_utf16("hello world test");
        let cursor = text.len();
        assert_eq!(
            word_start_before(&text, cursor),
            to_utf16("hello world ").len()
        );
    }

    #[test]
    fn skips_trailing_spaces_before_word() {
        let text = to_utf16("hello world   ");
        let cursor = text.len();
        assert_eq!(word_start_before(&text, cursor), to_utf16("hello ").len());
    }

    #[test]
    fn stops_at_start_of_text() {
        let text = to_utf16("hello");
        let cursor = text.len();
        assert_eq!(word_start_before(&text, cursor), 0);
    }

    #[test]
    fn cursor_in_middle_of_text() {
        let text = to_utf16("foo bar baz");
        let cursor = to_utf16("foo bar ").len();
        assert_eq!(word_start_before(&text, cursor), to_utf16("foo ").len());
    }

    #[test]
    fn cursor_at_zero_is_noop_boundary() {
        let text = to_utf16("hello");
        assert_eq!(word_start_before(&text, 0), 0);
    }

    #[test]
    fn stale_everything_reply_is_rejected_after_a_new_query() {
        let first = next_everything_reply_id(0);
        let second = next_everything_reply_id(first);

        assert!(!accepts_everything_reply(true, second, first));
        assert!(accepts_everything_reply(true, second, second));
        assert!(!accepts_everything_reply(false, second, second));
    }
}
