//! Chrome / Edge Native Messaging host。
//!
//! stdin/stdout のフレームを常駐部の `WM_COPYDATA` へ中継するだけの別プロセス。
//! 逆方向のタブ前面化要求は非表示ウィンドウで受け、同じ Native Messaging port
//! から拡張へ戻す。常駐部の表示・検索経路に stdio やブラウザ通信を持ち込まない。

use std::io::{self, Read, Write};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FindWindowW, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CLOSE, WM_COPYDATA, WM_DESTROY,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::w;

use waypoint::browser_tabs::{
    FOCUS_COPYDATA, HOST_WINDOW_CLASS, MAX_SNAPSHOT_BYTES, SNAPSHOT_COPYDATA,
};
use waypoint::tray::CLASS_NAME as TRAY_WINDOW_CLASS;

static OUTPUT: OnceLock<Mutex<io::Stdout>> = OnceLock::new();

fn main() {
    let Ok(window) = create_window() else {
        return;
    };
    // HWND は raw pointer を持つため Send ではない。値だけを整数として渡し、
    // reader スレッド側で戻す (PostMessageW にしか使わない)。
    let reader_window = window.0 as isize;
    std::thread::spawn(move || forward_snapshots(HWND(reader_window as *mut _)));

    unsafe {
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        while GetMessageW(&mut message, Some(HWND::default()), 0, 0).as_bool() {
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
        }
    }
}

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: HOST_WINDOW_CLASS,
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            HOST_WINDOW_CLASS,
            w!("waypoint tab host"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

fn forward_snapshots(window: HWND) {
    let mut input = io::stdin();
    while let Ok(Some(message)) = read_message(&mut input) {
        let forwarded = send_snapshot(&message);
        let _ = write_message(&serde_json::json!({
            "type": "ack",
            "connected": forwarded,
        }));
    }
    unsafe {
        let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native message exceeds limit",
        ));
    }
    let mut message = vec![0; length];
    input.read_exact(&mut message)?;
    Ok(Some(message))
}

fn write_message(value: &serde_json::Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let stdout = OUTPUT.get_or_init(|| Mutex::new(io::stdout()));
    let mut stdout = stdout
        .lock()
        .expect("native messaging stdout lock poisoned");
    stdout.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stdout.write_all(&bytes)?;
    stdout.flush()
}

fn send_snapshot(message: &[u8]) -> bool {
    let Ok(tray) = (unsafe { FindWindowW(TRAY_WINDOW_CLASS, None) }) else {
        return false;
    };
    let copy_data = COPYDATASTRUCT {
        dwData: SNAPSHOT_COPYDATA,
        cbData: message.len() as u32,
        lpData: message.as_ptr().cast_mut().cast(),
    };
    unsafe {
        SendMessageTimeoutW(
            tray,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM((&copy_data as *const COPYDATASTRUCT).cast::<()>() as isize),
            SMTO_ABORTIFHUNG,
            100,
            None,
        )
        .0 != 0
    }
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COPYDATA if lparam.0 != 0 => unsafe {
            let copy_data = &*(lparam.0 as *const COPYDATASTRUCT);
            if copy_data.dwData == FOCUS_COPYDATA
                && !copy_data.lpData.is_null()
                && copy_data.cbData > 0
                && copy_data.cbData as usize <= MAX_SNAPSHOT_BYTES
            {
                let message = std::slice::from_raw_parts(
                    copy_data.lpData.cast::<u8>(),
                    copy_data.cbData as usize,
                )
                .to_vec();
                std::thread::spawn(move || {
                    if let Ok(value) = serde_json::from_slice(&message) {
                        let _ = write_message(&value);
                    }
                });
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, message, WPARAM(0), lparam)
        },
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, WPARAM(0), lparam) },
    }
}
