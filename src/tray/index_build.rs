//! Quick Launch の検索インデックスのバックグラウンド構築。
//!
//! 起動時と設定の再読み込み時に、`Index::build` (スタートメニューの `.lnk`
//! COM 解決・ブラウザ履歴や Azure DevOps キャッシュの SQLite 読み取り) と
//! `dynamic::refresh` (Recent の COM 解決) を UI スレッドで同期実行していた。
//! 実測で 35〜91ms + 44ms、ログオン直後の自動起動ではディスクが冷えていて
//! さらに伸びる。その間メッセージループが回らないため、トップレベル
//! ウィンドウ宛てのブロードキャスト (`WM_SETTINGCHANGE` や ShellExecute の
//! DDE 探索) を `SendMessage` した他プロセスまで待たされ、起動時に
//! 「固まる」原因になっていた。
//!
//! ここでは構築を専用スレッドで行い、完了を `WM_INDEX_BUILT` で受け取って
//! UI スレッドでは差し替えるだけにする。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use super::{refresh_azure_devops, with_state};
use crate::config::Config;
use crate::dynamic::Menus;
use crate::quick_launch::Index;
use crate::quick_launch_window;

pub(crate) const WM_INDEX_BUILT: u32 = WM_APP + 12;

/// 最新の構築要求の番号。再読み込みが続いたとき、古い config で組んだ
/// 索引が後から届いて新しい方を上書きしないよう、番号が一致した結果だけ使う。
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// 構築が panic した場合は `None` を置く。通知を送らないと `apply` が走らず、
/// そこから始める Azure DevOps の同期まで止まってしまうため。
static RESULT: Mutex<Option<(u64, Option<Index>)>> = Mutex::new(None);

/// 索引の構築を別スレッドで始める。結果は `apply` で反映する。
///
/// Recent/Frequent と開いているウィンドウの列挙はここでは行わない。
/// 既存の `dynamic::refresh_async` に任せ、索引の差し替え時に
/// その時点の `AppState::dynamic` で組み直す (`install_index`)。
pub(crate) fn build_async(hwnd: HWND, config: Config) {
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let notify = hwnd.0 as isize;
    std::thread::spawn(move || {
        let started = crate::startup_timing::enabled().then(std::time::Instant::now);
        // apps::scan の IShellLink は STA を要求する
        let _com = crate::shell::ComGuard::new();
        // panic 時の記録は panic hook が行う
        let index = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Index::build(&config, &Menus::default())
        }))
        .ok();
        if let Some(started) = started {
            crate::startup_timing::mark_elapsed(
                "background index build finished",
                started.elapsed(),
            );
        }
        let mut slot = RESULT.lock().unwrap_or_else(|e| e.into_inner());
        // 後から始めた構築が先に終わっていたら、古い結果で上書きしない
        if slot.as_ref().is_none_or(|(stored, _)| *stored < generation) {
            *slot = Some((generation, index));
        }
        drop(slot);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(notify as *mut _)),
                WM_INDEX_BUILT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}

/// `WM_INDEX_BUILT` を受けて索引を差し替える。
pub(crate) fn apply(hwnd: HWND) {
    let taken = RESULT.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((generation, index)) = taken else {
        return;
    };
    if generation != GENERATION.load(Ordering::Acquire) {
        return;
    }
    if let Some(index) = index {
        with_state(|s| {
            let state = s.borrow();
            if let Some(state) = state.as_ref() {
                quick_launch_window::install_index(index, &state.config, &state.dynamic);
            }
        });
    }
    crate::startup_timing::mark("background index installed on UI thread");
    // Azure DevOps の同期は索引が揃ってから始める。先に同期結果が届くと、
    // 後から来た索引が構築時点の古いキャッシュで候補を上書きしてしまう
    refresh_azure_devops(hwnd);
}
