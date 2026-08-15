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
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FW_NORMAL, FW_SEMIBOLD, FillRect,
    GetMonitorInfoW, HBITMAP, HBRUSH, HDC, HFONT, InvalidateRect, LineTo,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, MoveToEx, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETCUEBANNER, EM_SETMARGINS, ODS_SELECTED, SetWindowTheme,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::SIID_LINK;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, EN_CHANGE, GetClientRect, GetWindowTextLengthW,
    GetWindowTextW, HMENU, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL,
    LB_SETITEMHEIGHT, LBN_DBLCLK, LBS_HASSTRINGS, LBS_NOTIFY, LBS_OWNERDRAWFIXED, MoveWindow,
    PostMessageW, RegisterClassW, SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowTextW,
    ShowWindow, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_DRAWITEM, WM_ERASEBKGND, WM_KEYDOWN, WM_PAINT, WM_SETFONT, WM_SIZE,
    WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
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

const BACKGROUND: COLORREF = rgb(13, 13, 13);
const SURFACE: COLORREF = rgb(30, 30, 30);
const SURFACE_HOVER: COLORREF = rgb(42, 42, 42);
const ACCENT: COLORREF = rgb(0, 120, 212);
const TEXT_PRIMARY: COLORREF = rgb(245, 245, 245);
const TEXT_SECONDARY: COLORREF = rgb(166, 166, 166);

pub const WM_QUICK_LAUNCH_EXECUTE: u32 = WM_APP + 4;
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
    visible_results: usize,
    dpi: u32,
    edit_font: Option<HFONT>,
    name_font: Option<HFONT>,
    detail_font: Option<HFONT>,
    background_brush: Option<HBRUSH>,
    surface_brush: Option<HBRUSH>,
}

pub fn configure(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        // インデックスの差し替えだけ借用内で行い、描画は借用を解放してから
        let has_window = {
            let mut state = state.borrow_mut();
            state.index = Index::build(config, dynamic);
            state.visible_results = config.settings.quick_launch.visible_results.clamp(12, 24);
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

/// Quick Launch の子コントロール宛てキーを通常の DispatchMessage より先に扱う。
pub fn handle_message(message: &windows::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    if message.message != WM_KEYDOWN {
        return false;
    }
    let belongs_to_quick_launch = STATE.with(|state| {
        let state = state.borrow();
        Some(message.hwnd) == state.edit || Some(message.hwnd) == state.list
    });
    if !belongs_to_quick_launch {
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
        0x0d => queue_selected(),
        _ => return false,
    }
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
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
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
        let cue = HSTRING::from("Search folders");
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            EM_SETCUEBANNER,
            Some(WPARAM(1)),
            Some(LPARAM(cue.as_ptr() as isize)),
        );
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
            let search_gutter = scale(32, dpi);
            unsafe {
                if let Some(edit) = edit {
                    let _ = MoveWindow(
                        edit,
                        padding + search_gutter,
                        padding + scale(6, dpi),
                        width - padding * 2 - search_gutter - scale(8, dpi),
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
fn update_results(state: &RefCell<State>) {
    // read_text も Win32 呼び出しなので借用の外で済ませる
    let edit = state.borrow().edit;
    let query = edit.map(read_text).unwrap_or_default();

    let (list, labels, has_results) = {
        let mut state = state.borrow_mut();
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
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_RESETCONTENT,
            None,
            None,
        );
        for label in &labels {
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
        let (dpi, background, surface) = STATE.with(|state| {
            let state = state.borrow();
            (state.dpi, state.background_brush, state.surface_brush)
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

            let icon_pen = CreatePen(PS_SOLID, scale(2, dpi).max(1), TEXT_SECONDARY);
            let old_pen = SelectObject(hdc, icon_pen.into());
            let icon_left = padding + scale(10, dpi);
            let icon_top = padding + scale(9, dpi);
            let icon_size = scale(11, dpi);
            let _ = Ellipse(
                hdc,
                icon_left,
                icon_top,
                icon_left + icon_size,
                icon_top + icon_size,
            );
            let _ = MoveToEx(
                hdc,
                icon_left + icon_size - scale(1, dpi),
                icon_top + icon_size - scale(1, dpi),
                None,
            );
            let _ = LineTo(
                hdc,
                icon_left + icon_size + scale(5, dpi),
                icon_top + icon_size + scale(5, dpi),
            );
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(icon_pen.into());
        }
        let _ = EndPaint(window, &paint);
    }
}

unsafe fn draw_list_item(draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let Some((entry, name_font, detail_font, dpi)) = STATE.with(|state| {
        let state = state.borrow();
        let entry = state.results.get(draw.itemID as usize)?.clone();
        Some((entry, state.name_font, state.detail_font, state.dpi))
    }) else {
        return;
    };

    unsafe {
        let selected = draw.itemState.0 & ODS_SELECTED.0 != 0;
        let background = CreateSolidBrush(if selected { SURFACE_HOVER } else { BACKGROUND });
        FillRect(draw.hDC, &draw.rcItem, background);
        let _ = DeleteObject(background.into());

        if selected {
            let accent = CreateSolidBrush(ACCENT);
            let accent_rect = RECT {
                left: draw.rcItem.left,
                top: draw.rcItem.top + scale(5, dpi),
                right: draw.rcItem.left + scale(3, dpi),
                bottom: draw.rcItem.bottom - scale(5, dpi),
            };
            FillRect(draw.hDC, &accent_rect, accent);
            let _ = DeleteObject(accent.into());
        }

        match entry.action {
            Action::OpenFolder(_) => draw_path_icon(draw.hDC, &entry.path, draw.rcItem, dpi),
            Action::FocusWindow(hwnd) => {
                draw_window_icon(draw.hDC, HWND(hwnd as *mut _), draw.rcItem, dpi)
            }
            Action::OpenUrl(_) => draw_stock_icon(draw.hDC, SIID_LINK, draw.rcItem, dpi),
        }
        SetBkMode(draw.hDC, TRANSPARENT);
        let text_left = draw.rcItem.left + scale(40, dpi);
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
            draw_text(draw.hDC, &entry.name, &mut rect);
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

unsafe fn draw_path_icon(hdc: HDC, path: &str, rect: RECT, dpi: u32) {
    let size = scale(18, dpi);
    let Some(bitmap) = crate::icon::bitmap_for_sized(path, size) else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

unsafe fn draw_window_icon(hdc: HDC, hwnd: HWND, rect: RECT, dpi: u32) {
    let size = scale(18, dpi);
    let Some(bitmap) = crate::icon::bitmap_for_window_sized(hwnd, size) else {
        return;
    };
    unsafe { draw_icon_bitmap(hdc, bitmap, rect, dpi, size) };
}

unsafe fn draw_stock_icon(
    hdc: HDC,
    id: windows::Win32::UI::Shell::SHSTOCKICONID,
    rect: RECT,
    dpi: u32,
) {
    let size = scale(18, dpi);
    let Some(bitmap) = crate::icon::bitmap_for_stock_sized(id, size) else {
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
            rect.left + scale(11, dpi),
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
