//! Edge のお気に入り編集 (FR-6.17) 用の Native Messaging host。
//!
//! 標準入出力のフレーム (4 バイト長 + JSON) を、編集画面 (`waypoint-settings.exe
//! --bookmarks`) が待ち受ける名前付きパイプへ中継するだけで、中身は解釈しない。
//! 編集画面が閉じている間は 1 秒ごとに接続を試み、届いたフレームは捨てる
//! (編集画面は接続のたびにスナップショットを頼み直すため、取りこぼしても困らない)。
//!
//! パイプは 1 本を両方向に使う。同期ハンドルだと同じハンドルへの読みと書きが
//! 直列になり、編集画面からの読み待ちの間はブラウザからの書き込みが止まるため、
//! overlapped I/O で開いて読みと書きを別スレッドで同時に走らせる。

use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::Duration;

use waypoint::bookmark_editor::{MAX_FRAME_BYTES, PIPE_PATH};

/// 編集画面とのパイプ (`HANDLE` の値)。閉じている間は `None`。
/// 書き込み (main スレッド) と閉じる処理 (中継スレッド) が重ならないよう Mutex で守る。
static PIPE: Mutex<Option<isize>> = Mutex::new(None);

fn main() {
    std::thread::spawn(relay_editor_to_browser);

    let mut input = io::stdin().lock();
    // stdin が閉じた (ブラウザが拡張を止めた) ら終わる
    while let Ok(Some(frame)) = read_frame(&mut input) {
        let guard = PIPE.lock().expect("pipe lock poisoned");
        if let Some(handle) = *guard {
            // 失敗したら中継スレッド側の読み取りも失敗し、そちらで閉じて再接続する
            let _ = pipe::write_all(handle, &with_length(&frame));
        }
    }
}

/// 編集画面への接続を保ち、編集画面から来たフレームを stdout へ流す。
fn relay_editor_to_browser() {
    let stdout = io::stdout();
    loop {
        let Ok(handle) = pipe::connect(PIPE_PATH) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        *PIPE.lock().expect("pipe lock poisoned") = Some(handle);

        while let Ok(frame) = read_pipe_frame(handle) {
            let mut out = stdout.lock();
            if out
                .write_all(&with_length(&frame))
                .and_then(|_| out.flush())
                .is_err()
            {
                // ブラウザ側が閉じた。main スレッドも stdin の EOF で終わる
                std::process::exit(0);
            }
        }

        *PIPE.lock().expect("pipe lock poisoned") = None;
        pipe::close(handle);
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn read_frame(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = checked_length(length)?;
    let mut frame = vec![0; length];
    input.read_exact(&mut frame)?;
    Ok(Some(frame))
}

fn read_pipe_frame(handle: isize) -> io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    pipe::read_exact(handle, &mut length)?;
    let mut frame = vec![0; checked_length(length)?];
    pipe::read_exact(handle, &mut frame)?;
    Ok(frame)
}

fn checked_length(header: [u8; 4]) -> io::Result<usize> {
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds limit",
        ));
    }
    Ok(length)
}

/// 長さ 4 バイトを前に付けた 1 フレーム。1 回の書き込みで送り、他の書き込みと混ざらないようにする。
fn with_length(body: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    frame
}

/// overlapped I/O で開いた名前付きパイプの読み書き。
mod pipe {
    use std::ffi::c_void;
    use std::io;

    use windows::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
    use windows::Win32::System::Threading::CreateEventW;
    use windows::core::{HSTRING, PCWSTR};

    fn handle(raw: isize) -> HANDLE {
        HANDLE(raw as *mut c_void)
    }

    pub fn connect(path: &str) -> io::Result<isize> {
        let path = HSTRING::from(path);
        let opened = unsafe {
            CreateFileW(
                &path,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        opened.map(|h| h.0 as isize).map_err(io::Error::other)
    }

    pub fn close(raw: isize) {
        unsafe {
            let _ = CloseHandle(handle(raw));
        }
    }

    pub fn read_exact(raw: isize, buffer: &mut [u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < buffer.len() {
            let read = transfer(raw, |overlapped| unsafe {
                ReadFile(
                    handle(raw),
                    Some(&mut buffer[offset..]),
                    None,
                    Some(overlapped),
                )
            })?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            offset += read;
        }
        Ok(())
    }

    pub fn write_all(raw: isize, buffer: &[u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < buffer.len() {
            let written = transfer(raw, |overlapped| unsafe {
                WriteFile(handle(raw), Some(&buffer[offset..]), None, Some(overlapped))
            })?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            offset += written;
        }
        Ok(())
    }

    /// overlapped の読み書きを 1 回発行し、完了まで待って転送バイト数を返す。
    fn transfer(
        raw: isize,
        start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
    ) -> io::Result<usize> {
        let event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(io::Error::other)?;
        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        let result = match start(&mut overlapped) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => Ok(()),
            Err(error) => Err(io::Error::other(error)),
        }
        .and_then(|()| {
            let mut transferred = 0u32;
            unsafe { GetOverlappedResult(handle(raw), &overlapped, &mut transferred, true) }
                .map(|()| transferred as usize)
                .map_err(io::Error::other)
        });
        unsafe {
            let _ = CloseHandle(event);
        }
        result
    }
}
