use super::super::{STATE, State, window_proc};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MSG, RegisterClassW, SendMessageW, WA_ACTIVE, WA_CLICKACTIVE,
    WM_ACTIVATE, WM_CHAR, WM_KEYDOWN, WNDCLASSW, WS_CHILD, WS_POPUP,
};
use windows::core::w;

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
