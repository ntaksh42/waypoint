//! 外部 IPC・ファイル I/O を描画スレッドから切り離す。

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub(crate) const WM_ICON_READY: u32 = WM_APP + 30;
const QUEUE_LIMIT: usize = 64;
/// 読み込みスレッド数。不達 UNC のシェルアイコン取得 (2.1 秒) やウィンドウ
/// アイコンの応答待ちなど遅い 1 件が、後続の全アイコンを塞がないようにする。
const WORKER_THREADS: usize = 3;
type Loader = Box<dyn FnOnce() -> Option<HBITMAP> + Send>;

struct Request {
    key: String,
    generation: u64,
    notify: isize,
    load: Loader,
}

struct Reply {
    key: String,
    generation: u64,
    bitmap: Option<isize>,
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(raw) = self.bitmap {
            unsafe {
                let _ = DeleteObject(HBITMAP(raw as *mut _).into());
            }
        }
    }
}

struct Worker {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    pending: HashSet<String>,
    generation: u64,
    notify: isize,
}

impl Worker {
    fn new() -> Self {
        let (requests, incoming) = mpsc::sync_channel::<Request>(QUEUE_LIMIT);
        let incoming = Arc::new(Mutex::new(incoming));
        let (outgoing, replies) = mpsc::channel();
        for _ in 0..WORKER_THREADS {
            let incoming = Arc::clone(&incoming);
            let outgoing = outgoing.clone();
            std::thread::spawn(move || {
                let _com = crate::shell::ComGuard::new();
                loop {
                    // 取り出す間だけロックする。読み込み中は他のスレッドが次を取れる
                    let next = incoming.lock().unwrap_or_else(|e| e.into_inner()).recv();
                    let Ok(request) = next else {
                        break;
                    };
                    let bitmap =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(request.load))
                            .ok()
                            .flatten()
                            .map(|bitmap| bitmap.0 as isize);
                    let reply = Reply {
                        key: request.key,
                        generation: request.generation,
                        bitmap,
                    };
                    if outgoing.send(reply).is_err() {
                        break;
                    }
                    if request.notify != 0 {
                        unsafe {
                            let _ = PostMessageW(
                                Some(HWND(request.notify as *mut _)),
                                WM_ICON_READY,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                    }
                }
            });
        }
        Self {
            requests,
            replies,
            pending: HashSet::new(),
            generation: 0,
            notify: 0,
        }
    }

    fn request(&mut self, key: &str, load: Loader) {
        if self.pending.contains(key) || self.pending.len() >= QUEUE_LIMIT {
            return;
        }
        let request = Request {
            key: key.to_owned(),
            generation: self.generation,
            notify: self.notify,
            load,
        };
        // キューが詰まっていても UI は待たない。次の描画で再試行する。
        if self.requests.try_send(request).is_ok() {
            self.pending.insert(key.to_owned());
        }
    }

    fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending.clear();
    }

    fn take_ready(&mut self) -> Vec<Reply> {
        let mut ready = Vec::new();
        while let Ok(reply) = self.replies.try_recv() {
            if reply.generation == self.generation {
                self.pending.remove(&reply.key);
                ready.push(reply);
            }
            // 破棄済みのキャッシュへの応答は Drop でビットマップも解放する。
        }
        ready
    }
}

thread_local! {
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

pub(crate) fn set_notify(hwnd: HWND) {
    WORKER.with(|worker| {
        worker.borrow_mut().get_or_insert_with(Worker::new).notify = hwnd.0 as isize;
    });
}

pub(super) fn request(key: &str, load: impl FnOnce() -> Option<HBITMAP> + Send + 'static) {
    WORKER.with(|worker| {
        worker
            .borrow_mut()
            .get_or_insert_with(Worker::new)
            .request(key, Box::new(load));
    });
}

pub(crate) fn apply_ready() {
    let ready = WORKER.with(|worker| {
        worker
            .borrow_mut()
            .as_mut()
            .map(Worker::take_ready)
            .unwrap_or_default()
    });
    for mut reply in ready {
        super::store_bitmap(
            &reply.key,
            reply.bitmap.take().map(|raw| HBITMAP(raw as *mut _)),
        );
    }
}

pub(super) fn clear() {
    WORKER.with(|worker| {
        if let Some(worker) = worker.borrow_mut().as_mut() {
            worker.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slow_loader_does_not_block_or_duplicate_requests() {
        let mut worker = Worker::new();
        let (started, start) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        worker.request(
            "slow",
            Box::new(move || {
                started.send(()).unwrap();
                finished.recv().unwrap();
                None
            }),
        );
        start.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.request("slow", Box::new(|| panic!("duplicate loader")));
        assert_eq!(worker.pending.len(), 1);
        // 遅い 1 件が残っていても、後続は別のスレッドが先に返す
        worker.request("after", Box::new(|| None));
        let first = worker.replies.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.key, "after");
        assert!(worker.take_ready().len() <= 1);
        finish.send(()).unwrap();
        let reply = worker.replies.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reply.key, "slow");
        assert!(reply.bitmap.is_none());
    }

    #[test]
    fn clearing_cache_discards_old_replies_without_removing_new_requests() {
        let mut worker = Worker::new();
        let (outgoing, incoming) = mpsc::channel();
        worker.replies = incoming;
        let bitmap = super::super::convert::rgba_to_bitmap(
            &[255, 0, 0, 255],
            windows::Win32::Foundation::SIZE { cx: 1, cy: 1 },
        )
        .unwrap();
        outgoing
            .send(Reply {
                key: "icon".into(),
                generation: 0,
                bitmap: Some(bitmap.0 as isize),
            })
            .unwrap();
        worker.clear();
        worker.pending.insert("icon".into());
        assert!(worker.take_ready().is_empty());
        assert!(worker.pending.contains("icon"));
        unsafe {
            let mut object = windows::Win32::Graphics::Gdi::BITMAP::default();
            assert_eq!(
                windows::Win32::Graphics::Gdi::GetObjectW(
                    bitmap.into(),
                    size_of_val(&object) as i32,
                    Some(std::ptr::from_mut(&mut object).cast()),
                ),
                0
            );
        }
    }

    #[test]
    fn full_queue_does_not_block_and_can_retry_after_completion() {
        let mut worker = Worker::new();
        let (finish, finished) = mpsc::channel();
        worker.request(
            "blocked",
            Box::new(move || {
                finished.recv().unwrap();
                None
            }),
        );
        for index in 1..QUEUE_LIMIT {
            worker.request(&index.to_string(), Box::new(|| None));
        }
        worker.request("overflow", Box::new(|| None));
        assert_eq!(worker.pending.len(), QUEUE_LIMIT);
        assert!(!worker.pending.contains("overflow"));
        finish.send(()).unwrap();
        for _ in 0..QUEUE_LIMIT {
            let reply = worker.replies.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.pending.remove(&reply.key);
        }
        worker.request("overflow", Box::new(|| None));
        assert_eq!(
            worker
                .replies
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .key,
            "overflow"
        );
    }
}
