use super::activate_window;
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IsIconic, SW_MINIMIZE, ShowWindow, WS_POPUP,
};
use windows::core::w;

#[test]
fn activating_unresponsive_minimized_window_does_not_wait() {
    let (created, window) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let owner = std::thread::spawn(move || unsafe {
        let hwnd = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Waypoint activation test"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
        assert!(IsIconic(hwnd).as_bool());
        created.send(hwnd.0 as isize).unwrap();
        // 復元要求を処理しない相手を再現する。失敗時も解放して終了する。
        released.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = DestroyWindow(hwnd);
    });
    let raw = window.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done, completed) = mpsc::channel();
    let caller = std::thread::spawn(move || {
        activate_window(HWND(raw as *mut _));
        done.send(()).unwrap();
    });
    let result = completed.recv_timeout(Duration::from_millis(250));
    release.send(()).unwrap();
    owner.join().unwrap();
    caller.join().unwrap();
    assert!(result.is_ok(), "activation waited for the target window");
}

#[test]
fn activating_responsive_minimized_window_restores_it() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW};
    let (created, window) = mpsc::channel();
    let (finish, finished) = mpsc::channel();
    let (restored, restoration) = mpsc::channel();
    let owner = std::thread::spawn(move || unsafe {
        let hwnd = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Waypoint restore test"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
        assert!(IsIconic(hwnd).as_bool());
        created.send(hwnd.0 as isize).unwrap();
        let mut reported = false;
        while finished.try_recv().is_err() {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            if !reported && !IsIconic(hwnd).as_bool() {
                restored.send(()).unwrap();
                reported = true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = DestroyWindow(hwnd);
    });
    let raw = window.recv_timeout(Duration::from_secs(5)).unwrap();
    activate_window(HWND(raw as *mut _));
    let result = restoration.recv_timeout(Duration::from_secs(5));
    finish.send(()).unwrap();
    owner.join().unwrap();
    assert!(result.is_ok(), "target window was not restored");
}
