//! 標準 Win32 コントロールだけで構成する Quick Launch 画面。

mod badge;
mod dispatch;
mod draw;
mod draw_icons;
mod input;
mod layout;
mod search;
#[cfg(test)]
mod tests;

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HMENU, LBS_HASSTRINGS, LBS_NOTIFY, LBS_OWNERDRAWVARIABLE,
    RegisterClassW, SW_SHOW, SetForegroundWindow, SetWindowTextW, ShowWindow, WINDOW_STYLE, WM_APP,
    WM_KEYDOWN, WM_SYSKEYDOWN, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, Result, w};

use crate::config::Config;
use crate::dynamic::Menus;
use crate::quick_launch::{Entry, Index};
use dispatch::dispatch;
use input::{
    add_selected_to_favorites, copy_selected_path, delete_word_before_cursor, first_selectable_row,
    hide_window, last_selectable_row, move_selection, queue_selected, reveal_selected_in_explorer,
    select_at,
};
use layout::{apply_dpi, apply_window_chrome, position_window};
use search::update_results;

const EDIT_ID: isize = 1001;
const LIST_ID: isize = 1002;
const WINDOW_WIDTH: i32 = 720;
const PADDING: i32 = 10;
const EDIT_HEIGHT: i32 = 34;
const ROW_HEIGHT: i32 = 42;
/// セクション見出し行の高さ。通常項目より詰めて、区切りだと分かる程度にする。
const HEADER_HEIGHT: i32 = 26;
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
const SURFACE: COLORREF = rgb(32, 30, 28);
const SURFACE_HOVER: COLORREF = rgb(44, 41, 38);
const ACCENT: COLORREF = rgb(111, 168, 201);
const TEXT_PRIMARY: COLORREF = rgb(245, 245, 245);
const TEXT_SECONDARY: COLORREF = rgb(166, 166, 166);
/// breadcrumb 用。detail 行の中でもさらに一段控えめにする (FR-9.6 の補助情報)。
const TEXT_MUTED: COLORREF = rgb(117, 112, 106);

pub const WM_QUICK_LAUNCH_EXECUTE: u32 = WM_APP + 4;
/// Everything からの検索結果を識別する `WM_COPYDATA` の `dwData` の初期値。
/// クエリごとに増やし、入力前の古い応答を判別できるようにする。
const EVERYTHING_REPLY_ID_START: u32 = WM_APP + 5;
/// `Ctrl+Shift+Enter` で選択項目を config へ登録するよう常駐部へ依頼する。
pub const WM_QUICK_LAUNCH_ADD_TO_FAVORITES: u32 = WM_APP + 6;
/// Azure DevOps の Work Item 検索スレッドが結果を返す通知。
pub const WM_QUICK_LAUNCH_AZURE_RESULTS: u32 = WM_APP + 7;
/// Quick Launch が一度に Everything へ要求する最大件数。
/// 全件表示はしない (`visible_results` の上限と同じ枠で足りる)。
const EVERYTHING_MAX_RESULTS: u32 = 24;
const CLASS_NAME: PCWSTR = w!("WaypointQuickLaunchWindow");

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// リストボックスの 1 行が何を表すか。`results` のインデックスと行番号の
/// ずれを吸収するため、行番号から `results` を直接インデックス参照する
/// 代わりに必ずこの配列 (`State::rows`) を経由する。
#[derive(Debug, Clone, Copy)]
enum RowKind {
    /// `results[usize]` を表示する通常の項目行。選択・実行の対象。
    Item(usize),
    /// 絞り込みなし一覧の区分見出し。選択・実行の対象外。
    Header(&'static str),
    /// 検索中・0 件時の説明文 (`State::empty_message`)。選択・実行の対象外。
    Message,
}

#[derive(Default)]
struct State {
    window: Option<HWND>,
    edit: Option<HWND>,
    list: Option<HWND>,
    owner: Option<HWND>,
    origin: Option<HWND>,
    index: Index,
    /// 拡張から受け取った現在のブラウザタブ。Index を再構築しても失わないよう、
    /// 検索インデックスとは別にメモリ上で保持する。
    browser_tabs: Vec<(crate::browser_tabs::Browser, crate::browser_tabs::Tab)>,
    results: Vec<Entry>,
    /// 直前に同期検索した入力。末尾への文字追加だけなら、前回の候補を
    /// 起点に再検索して全索引の走査を避けるために使う。
    previous_query: Option<String>,
    /// リストボックスの行番号ごとの内訳。通常検索時は `Item(0), Item(1), ...`
    /// のフラットな並び (見出しなし)。行番号と `results` の対応を一箇所の
    /// 配列に固定することで、描画・選択移動・実行の各所で見出し行と項目行の
    /// 変換ロジックを重複させない。
    rows: Vec<RowKind>,
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
    azure_devops: crate::config::AzureDevOpsSettings,
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
    /// `az wit ` 中だけ立てる。古い検索スレッドの結果を捨てるために使う。
    azure_work_items_active: bool,
    azure_work_item_reply_id: u32,
    azure_work_item_query: String,
    /// PR のライブ検索 (`AzureLivePullRequestSearch`) 中だけ立てる。
    /// `azure_work_item_*` と同じ役割だが、Work Item のライブ検索と
    /// reply_id の名前空間を分けるために独立させている。
    azure_pull_requests_live_active: bool,
    azure_pull_request_reply_id: u32,
    /// 非同期検索中・0 件時に結果一覧へ出す説明。実行対象にはしない。
    empty_message: Option<String>,
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
            let tabs = state.browser_tabs.clone();
            state.index.set_browser_tabs(&tabs);
            state.previous_query = None;
            state.visible_results = config.settings.quick_launch.visible_results.clamp(12, 24);
            state.everything_enabled = config.settings.quick_launch.include_everything;
            state.azure_devops = config.settings.quick_launch.azure_devops.clone();
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// Native Messaging host が届けた 1 ブラウザ分のタブ一覧を入れ替える。
///
/// タブの変更通知時だけ呼ばれる。Quick Launch が表示中なら、現在の `t ` 検索結果も
/// 即座に差し替えるが、ブラウザへ同期問い合わせは行わない。
pub fn replace_browser_tabs(
    browser: crate::browser_tabs::Browser,
    tabs: Vec<crate::browser_tabs::Tab>,
) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            state.browser_tabs.retain(|(source, _)| *source != browser);
            state
                .browser_tabs
                .extend(tabs.into_iter().map(|tab| (browser, tab)));
            let tabs = state.browser_tabs.clone();
            state.index.set_browser_tabs(&tabs);
            state.previous_query = None;
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
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(window), None, true);
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
        // 見出し行 (区分見出し) は選択対象外。results.len() を直接使うと
        // 見出しの数だけ結果がずれる不具合になる (実測で End が最終項目
        // 手前に着地した)。first/last_selectable_row で行番号ベースに揃える。
        0x24 => {
            let (list, rows) = STATE.with(|state| {
                let state = state.borrow();
                (state.list, state.rows.clone())
            });
            if let Some(row) = first_selectable_row(&rows) {
                select_at(list, row);
            }
        }
        0x23 => {
            let (list, rows) = STATE.with(|state| {
                let state = state.borrow();
                (state.list, state.rows.clone())
            });
            if let Some(row) = last_selectable_row(&rows) {
                select_at(list, row);
            }
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
        search::invalidate_search_bar(window, dpi);
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
                | WINDOW_STYLE((LBS_NOTIFY | LBS_OWNERDRAWVARIABLE | LBS_HASSTRINGS) as u32),
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

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}
