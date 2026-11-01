//! 実際の検索窓・Enter 処理・トレイの実行通知まで通す実機テスト。

use super::super::{STATE, State, handle_message, show};
use crate::dynamic::{Menus, WindowEntry};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, IsIconic, MSG,
    PM_REMOVE, PeekMessageW, SW_HIDE, SW_MINIMIZE, SW_SHOW, SetForegroundWindow, SetWindowTextW,
    ShowWindow, TranslateMessage, WM_KEYDOWN, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

fn pump_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if !handle_message(&message) {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires selecting the Waypoint switch test control window on the desktop"]
fn w_search_enter_foregrounds_normal_minimized_and_modal_windows() {
    unsafe {
        let create = |title, owner| {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                title,
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                360,
                160,
                owner,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let target = create(w!("Waypoint switch target"), None);
        let dialog = create(w!("Waypoint switch dialog"), Some(target));
        let control = create(w!("Waypoint switch test control"), None);
        let owner = crate::tray::create_window().unwrap();
        let _ = ShowWindow(target, SW_SHOW);
        let _ = ShowWindow(control, SW_SHOW);
        println!("Select 'Waypoint switch test control' to start the w/Enter test");
        let ready = pump_until(|| GetForegroundWindow() == control, Duration::from_secs(45));
        // 操作ツールが前面化を終えてから測る。途中で次の窓へ進めると、
        // ツールの前面化リトライ自体が測定対象からフォーカスを奪う。
        pump_until(|| false, Duration::from_secs(1));
        let config = crate::config::Config::default();
        let menus = Menus {
            all_windows: vec![WindowEntry {
                title: "Waypoint switch target".into(),
                hwnd: target.0 as isize,
                process_name: "waypoint-test.exe".into(),
            }],
            ..Default::default()
        };
        STATE.with(|state| state.borrow_mut().index.refresh_dynamic(&config, &menus));
        let mut outcomes = Vec::new();
        if ready {
            for mode in ["normal", "minimized", "modal"]
                .into_iter()
                .cycle()
                .take(30)
            {
                let _ = EnableWindow(target, true);
                let _ = ShowWindow(dialog, SW_HIDE);
                let expected = if mode == "modal" {
                    let _ = EnableWindow(target, false);
                    let _ = ShowWindow(dialog, SW_SHOW);
                    dialog
                } else {
                    if mode == "minimized" {
                        let _ = ShowWindow(target, SW_MINIMIZE);
                    }
                    target
                };
                // 最小化・ダイアログ終了の通知を処理してから検索窓を開く。
                // 表示前の通知が残った状態へ人工的に Enter を送らない。
                pump_until(|| false, Duration::from_millis(250));
                let _ = SetForegroundWindow(control);
                show(owner, Some(control)).unwrap();
                let launcher = STATE.with(|state| state.borrow().window.unwrap());
                assert!(
                    pump_until(|| GetForegroundWindow() == launcher, Duration::from_secs(1)),
                    "search window was not ready for keyboard input"
                );
                let edit = STATE.with(|state| state.borrow().edit.unwrap());
                let _ = SetWindowTextW(edit, w!("w Waypoint switch target"));
                assert!(handle_message(&MSG {
                    hwnd: edit,
                    message: WM_KEYDOWN,
                    wParam: WPARAM(0x0d),
                    ..Default::default()
                }));
                let mut succeeded = pump_until(
                    || GetForegroundWindow() == expected && !IsIconic(target).as_bool(),
                    Duration::from_secs(3),
                );
                pump_until(|| false, Duration::from_millis(100));
                succeeded &= GetForegroundWindow() == expected;
                println!(
                    "{mode}: foreground={:?}, expected={expected:?}, iconic={}, passed={succeeded}",
                    GetForegroundWindow(),
                    IsIconic(target).as_bool()
                );
                outcomes.push((mode, succeeded));
            }
        }
        let window = STATE.with(|state| state.borrow().window);
        if let Some(window) = window {
            let _ = DestroyWindow(window);
        }
        STATE.with(|state| *state.borrow_mut() = State::default());
        let _ = DestroyWindow(dialog);
        let _ = EnableWindow(target, true);
        let _ = DestroyWindow(target);
        let _ = DestroyWindow(control);
        let _ = DestroyWindow(owner);
        assert!(ready, "test control was never activated");
        assert!(outcomes.iter().all(|(_, passed)| *passed), "{outcomes:?}");
    }
}

#[test]
#[ignore = "requires running Files and Orca apps, and pressing Space in the test control window"]
fn w_file_foregrounds_running_files_app() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    unsafe {
        let _com = crate::shell::ComGuard::new();
        let menus = crate::dynamic::refresh();
        let files: Vec<_> = menus
            .all_windows
            .iter()
            .filter(|entry| entry.process_name.eq_ignore_ascii_case("Files.exe"))
            .collect();
        assert_eq!(
            files.len(),
            1,
            "expected exactly one running Files window: {files:?}"
        );
        let expected = HWND(files[0].hwnd as *mut _);
        let owner = crate::tray::create_window().unwrap();
        let control = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Waypoint Files switch control"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            360,
            160,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let config = crate::config::Config::default();
        STATE.with(|state| state.borrow_mut().index.refresh_dynamic(&config, &menus));
        let _ = ShowWindow(control, SW_SHOW);
        let mut input = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            ..Default::default()
        };
        assert!(GetLastInputInfo(&mut input).as_bool());
        let started = input.dwTime;
        println!("Press Space in 'Waypoint Files switch control' to start the real Files test");
        let ready = pump_until(
            || {
                let _ = GetLastInputInfo(&mut input);
                GetForegroundWindow() == control && input.dwTime != started
            },
            Duration::from_secs(45),
        );
        pump_until(|| false, Duration::from_secs(1));
        let origin = FindWindowW(None, w!("Orca")).unwrap();
        let mut outcomes = Vec::new();
        if ready {
            for _ in 0..10 {
                let _ = ShowWindow(control, SW_SHOW);
                let _ = SetForegroundWindow(control);
                SetWindowPos(
                    origin,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                )
                .unwrap();
                show(owner, Some(origin)).unwrap();
                let _ = ShowWindow(control, SW_HIDE);
                let launcher = STATE.with(|state| state.borrow().window.unwrap());
                assert_eq!(
                    GetForegroundWindow(),
                    launcher,
                    "Quick Launch must own the foreground before selection"
                );
                let edit = STATE.with(|state| state.borrow().edit.unwrap());
                let _ = SetWindowTextW(edit, w!("w file"));
                println!(
                    "w file: foreground={:?}, results={:?}",
                    GetForegroundWindow(),
                    STATE.with(|state| state
                        .borrow()
                        .results
                        .iter()
                        .map(|entry| entry.action.clone())
                        .collect::<Vec<_>>())
                );
                assert!(handle_message(&MSG {
                    hwnd: edit,
                    message: WM_KEYDOWN,
                    wParam: WPARAM(0x0d),
                    ..Default::default()
                }));
                let mut succeeded =
                    pump_until(|| GetForegroundWindow() == expected, Duration::from_secs(2));
                pump_until(|| false, Duration::from_millis(250));
                succeeded &= GetForegroundWindow() == expected;
                println!(
                    "w file: foreground={:?}, expected={expected:?}, passed={succeeded}",
                    GetForegroundWindow()
                );
                outcomes.push(succeeded);
            }
        }
        if let Some(window) = STATE.with(|state| state.borrow().window) {
            let _ = DestroyWindow(window);
        }
        STATE.with(|state| *state.borrow_mut() = State::default());
        let _ = DestroyWindow(control);
        let _ = DestroyWindow(owner);
        assert!(ready, "test control was never activated");
        assert!(outcomes.iter().all(|passed| *passed), "{outcomes:?}");
    }
}
