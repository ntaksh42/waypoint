//! 標準 Win32 コントロールだけで構成する Quick Launch 画面。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, DEFAULT_GUI_FONT, GetMonitorInfoW, GetStockObject, GetSysColorBrush,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, EN_CHANGE, GetClientRect, GetWindowTextLengthW,
    GetWindowTextW, HMENU, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL,
    LB_SETITEMHEIGHT, LBN_DBLCLK, MoveWindow, PostMessageW, RegisterClassW, SW_HIDE, SW_SHOW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_KEYDOWN, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::config::{Config, OpenMode};
use crate::dynamic::Menus;
use crate::quick_launch::{Entry, Index};

const EDIT_ID: isize = 1001;
const LIST_ID: isize = 1002;
const PADDING: i32 = 10;
const EDIT_HEIGHT: i32 = 28;
const ROW_HEIGHT: i32 = 22;
const WINDOW_WIDTH: i32 = 720;

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
    pending: Option<(Entry, OpenMode)>,
    visible_results: usize,
}

pub fn configure(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.index = Index::build(config, dynamic);
        state.visible_results = config.settings.quick_launch.visible_results.clamp(5, 20);
        if state.window.is_some() {
            update_results(&mut state);
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
    position_window(window, origin.unwrap_or(owner), visible_results);
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        let _ = SetFocus(Some(edit));
    }
    Ok(())
}

pub fn take_pending() -> Option<(Entry, OpenMode, Option<HWND>)> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let origin = state.origin;
        state
            .pending
            .take()
            .map(|(entry, mode)| (entry, mode, origin))
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
        0x26 => STATE.with(|state| move_selection(&state.borrow(), -1)),
        0x28 => STATE.with(|state| move_selection(&state.borrow(), 1)),
        0x24 => STATE.with(|state| select_at(&state.borrow(), 0)),
        0x23 => STATE.with(|state| {
            let state = state.borrow();
            select_at(&state, state.results.len().saturating_sub(1));
        }),
        0x21 => STATE.with(|state| move_selection(&state.borrow(), -10)),
        0x22 => STATE.with(|state| move_selection(&state.borrow(), 10)),
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
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
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
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER,
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
                | WS_BORDER
                | WS_VSCROLL
                | WINDOW_STYLE(windows::Win32::UI::WindowsAndMessaging::LBS_NOTIFY as u32),
            PADDING,
            PADDING + EDIT_HEIGHT + PADDING,
            WINDOW_WIDTH - PADDING * 2,
            260,
            Some(window),
            Some(HMENU(LIST_ID as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let font = GetStockObject(DEFAULT_GUI_FONT);
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_SETITEMHEIGHT,
            Some(WPARAM(0)),
            Some(LPARAM(ROW_HEIGHT as isize)),
        );
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.window = Some(window);
            state.edit = Some(edit);
            state.list = Some(list);
            state.owner = Some(owner);
        });
    }
    Ok(())
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            let notification = ((wparam.0 >> 16) & 0xffff) as u32;
            let control = HWND(lparam.0 as *mut _);
            let is_edit = STATE.with(|state| Some(control) == state.borrow().edit);
            let is_list = STATE.with(|state| Some(control) == state.borrow().list);
            if is_edit && notification == EN_CHANGE {
                STATE.with(|state| {
                    update_results(&mut state.borrow_mut());
                });
            } else if is_list && notification == LBN_DBLCLK {
                queue_selected();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            STATE.with(|state| {
                let state = state.borrow();
                unsafe {
                    if let Some(edit) = state.edit {
                        let _ = MoveWindow(
                            edit,
                            PADDING,
                            PADDING,
                            width - PADDING * 2,
                            EDIT_HEIGHT,
                            true,
                        );
                    }
                    if let Some(list) = state.list {
                        let top = PADDING + EDIT_HEIGHT + PADDING;
                        let _ = MoveWindow(
                            list,
                            PADDING,
                            top,
                            width - PADDING * 2,
                            height - top - PADDING,
                            true,
                        );
                    }
                }
            });
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

fn update_results(state: &mut State) {
    let query = state.edit.map(read_text).unwrap_or_default();
    state.results = state.index.search(&query).into_iter().cloned().collect();
    let Some(list) = state.list else {
        return;
    };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_RESETCONTENT,
            None,
            None,
        );
        for entry in &state.results {
            let context = if entry.breadcrumb.is_empty() {
                entry.path.clone()
            } else {
                format!("{}  —  {}", entry.breadcrumb, entry.path)
            };
            let label = HSTRING::from(format!("{}    {}", entry.name, context));
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        if !state.results.is_empty() {
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

fn current_selection(state: &State) -> Option<usize> {
    let list = state.list?;
    let selected = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageW(list, LB_GETCURSEL, None, None).0
    };
    (selected >= 0).then_some(selected as usize)
}

fn select_at(state: &State, index: usize) {
    let Some(list) = state.list else {
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

fn move_selection(state: &State, delta: isize) {
    if state.results.is_empty() {
        return;
    }
    let current = current_selection(state).unwrap_or(0);
    let next = current
        .saturating_add_signed(delta)
        .min(state.results.len().saturating_sub(1));
    select_at(state, next);
}

fn queue_selected() {
    let queued = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let entry = current_selection(&state)
            .and_then(|index| state.results.get(index))
            .cloned()?;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let mode = if shift {
            OpenMode::NewWindow
        } else if control {
            OpenMode::Reuse
        } else {
            entry.open
        };
        state.pending = Some((entry, mode));
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

fn position_window(window: HWND, monitor_window: HWND, rows: usize) {
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
        let height = PADDING * 3 + EDIT_HEIGHT + ROW_HEIGHT * rows as i32 + 32;
        let x = work.left + (work.right - work.left - WINDOW_WIDTH) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
        let _ = MoveWindow(window, x, y, WINDOW_WIDTH, height, true);
    }
}
