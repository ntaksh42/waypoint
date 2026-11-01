use super::super::{STATE, State, window_proc};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MSG, RegisterClassW, SendMessageW, WA_ACTIVE, WA_CLICKACTIVE,
    WM_ACTIVATE, WM_CHAR, WM_KEYDOWN, WNDCLASSW, WS_CHILD, WS_POPUP,
};
use windows::core::w;

thread_local! {
    static NEXT_ACTIVE: std::cell::Cell<Option<isize>> = const { std::cell::Cell::new(None) };
}

extern "system" fn switching_window_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    if message == WM_ACTIVATE && wparam.0 & 0xffff == 0 {
        NEXT_ACTIVE.with(|next| {
            if next.get().is_none() {
                next.set(Some(lparam.0));
            }
        });
    }
    window_proc(hwnd, message, wparam, lparam)
}

#[test]
fn selecting_window_transfers_activation_directly_to_target() {
    use crate::quick_launch::{Action, Entry};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, SetActiveWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        LB_ADDSTRING, LB_SETCURSEL, SW_SHOW, ShowWindow, WS_OVERLAPPEDWINDOW,
    };
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            lpfnWndProc: Some(switching_window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("WaypointSwitchOrderTest"),
            ..Default::default()
        };
        assert_ne!(RegisterClassW(&class), 0);
        let target = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Waypoint switch order target"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            200,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let origin = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Waypoint switch order origin"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            200,
            100,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let launcher = CreateWindowExW(
            Default::default(),
            class.lpszClassName,
            w!("Waypoint switch order launcher"),
            WS_POPUP,
            0,
            0,
            200,
            100,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let list = CreateWindowExW(
            Default::default(),
            w!("LISTBOX"),
            w!(""),
            WS_CHILD,
            0,
            0,
            100,
            100,
            Some(launcher),
            None,
            None,
            None,
        )
        .unwrap();
        SendMessageW(
            list,
            LB_ADDSTRING,
            None,
            Some(LPARAM(w!("target").as_ptr() as isize)),
        );
        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), None);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.window = Some(launcher);
            state.list = Some(list);
            state.results = vec![Entry {
                name: "target".into(),
                breadcrumb: String::new(),
                path: String::new(),
                action: Action::FocusWindow(target.0 as isize),
                branch: None,
                azure: None,
            }];
            state.rows = vec![super::super::RowKind::Item(0)];
        });
        let _ = ShowWindow(target, SW_SHOW);
        let _ = ShowWindow(origin, SW_SHOW);
        let _ = ShowWindow(launcher, SW_SHOW);
        let _ = SetActiveWindow(launcher);
        assert_eq!(GetActiveWindow(), launcher);
        NEXT_ACTIVE.with(|next| next.set(None));
        super::super::input::queue_selected();
        let next = NEXT_ACTIVE.with(|next| next.get());
        let active = GetActiveWindow();
        let _ = DestroyWindow(launcher);
        let _ = DestroyWindow(origin);
        let _ = DestroyWindow(target);
        STATE.with(|state| *state.borrow_mut() = State::default());
        assert_eq!(
            next,
            Some(target.0 as isize),
            "selection reactivated another window before the target"
        );
        assert_eq!(active, target, "hiding the launcher reactivated the origin");
    }
}

#[test]
fn delayed_activation_keeps_keyboard_input_in_search_box() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("WaypointFocusRegressionTest"),
            ..Default::default()
        };
        assert_ne!(RegisterClassW(&class), 0);
        let window = CreateWindowExW(
            Default::default(),
            class.lpszClassName,
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let edit = CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD,
            0,
            0,
            200,
            24,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let list = CreateWindowExW(
            Default::default(),
            w!("LISTBOX"),
            w!(""),
            WS_CHILD,
            0,
            0,
            1,
            1,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.window = Some(window);
            state.edit = Some(edit);
            state.list = Some(list);
        });
        for activation in [WA_ACTIVE, WA_CLICKACTIVE] {
            let _ = SetFocus(Some(edit));
            assert_eq!(GetFocus(), edit);
            // show() の SetFocus の後にアクティブ化通知が届く順序を再現する。
            SendMessageW(
                window,
                WM_ACTIVATE,
                Some(WPARAM(activation as usize)),
                Some(LPARAM(0)),
            );
            assert_eq!(GetFocus(), edit, "activation moved focus away from search");
        }
        let _ = SetFocus(Some(window));
        assert_eq!(GetFocus(), edit, "top-level focus was not redirected");
        SendMessageW(
            GetFocus(),
            WM_CHAR,
            Some(WPARAM('x' as usize)),
            Some(LPARAM(1)),
        );
        assert_eq!(super::super::input::read_text(edit), "x");
        // リストを直接クリックした場合のフォーカスは維持する。
        let _ = SetFocus(Some(list));
        assert_eq!(GetFocus(), list);
        let _ = SetFocus(Some(window));
        assert!(super::super::handle_message(&MSG {
            hwnd: GetFocus(),
            message: WM_KEYDOWN,
            wParam: WPARAM(0x1b),
            ..Default::default()
        }));
        let _ = DestroyWindow(window);
        STATE.with(|state| *state.borrow_mut() = State::default());
    }
}
