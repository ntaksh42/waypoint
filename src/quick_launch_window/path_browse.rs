//! パス入力・フォルダ候補での Tab 操作 (FR-9.24〜FR-9.26)。
//!
//! 列挙 (`read_dir`) は I/O で、切断済みドライブや不達の UNC は SMB のタイムアウトまで
//! 返らない。UI スレッドでは呼ばず、Tab の明示操作のときだけ別スレッドで行い、
//! `WM_QUICK_LAUNCH_PATH_LIST` で結果を受け取る。打鍵では列挙しない (FR-9.4)。

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SendMessageW, SetWindowTextW};
use windows::core::HSTRING;

use super::input::{current_selection, entry_at_row, read_text};
use super::search::update_results;
use super::{STATE, State, WM_QUICK_LAUNCH_PATH_LIST};
use crate::config::OpenMode;
use crate::quick_launch::{
    Action, Entry, common_prefix, dir_entry, dirs_matching, normalize_path_input, parent_path,
    prefix_badge, same_dir, split_dir_prefix,
};

/// 1 回の列挙で保持する最大件数。
const MAX_DIRS: usize = 200;
/// ファイル属性 `FILE_ATTRIBUTE_SYSTEM`。`$Recycle.Bin` や互換用ジャンクションを除く。
const ATTRIBUTE_SYSTEM: u32 = 0x4;

/// 列挙済みのディレクトリ一覧 (`dir` は末尾 `\` 付き)。同じ親の間は使い回す。
pub(in crate::quick_launch_window) struct PathList {
    pub dir: String,
    pub names: Vec<String>,
}

/// Tab で候補を巡回している最中の状態。`text` は直近に検索欄へ書いたパスで、
/// 入力がそれと一致する間だけ巡回が続いているとみなす。
pub(in crate::quick_launch_window) struct PathCycle {
    pub dir: String,
    pub matches: Vec<String>,
    pub index: Option<usize>,
    pub text: String,
}

/// 最新の列挙要求の ID。古いスレッドは結果を残さず捨てる。
static LATEST_ID: AtomicU32 = AtomicU32::new(0);
static REPLY: Mutex<Option<(u32, String, Vec<String>)>> = Mutex::new(None);

/// 入力がパスでなくなったら閲覧状態を捨て、巡回中でなくなっていれば巡回だけ捨てる。
pub(super) fn sync(state: &RefCell<State>, query: &str) {
    let mut state = state.borrow_mut();
    match normalize_path_input(query) {
        None => {
            state.path_browse = false;
            state.path_cycle = None;
            state.path_tab_pending = None;
        }
        Some(path) => {
            if state.path_cycle.as_ref().is_some_and(|c| c.text != path) {
                state.path_cycle = None;
            }
        }
    }
}

/// ウィンドウを開くたびに列挙結果を捨てる (フォルダの中身は変わりうる)。
pub(super) fn reset(state: &mut State) {
    state.path_list = None;
    state.path_list_requested = None;
    state.path_browse = false;
    state.path_cycle = None;
    state.path_tab_pending = None;
}

/// パス入力時の候補を組み立てる。`results` には通常検索の結果が入っている。
///
/// 閲覧中・巡回中は検索結果の代わりに列挙したサブディレクトリを並べ、そうでなければ
/// 確定候補 (FR-9.24) を先頭に足すだけ。戻り値は選択すべき行 (巡回中のみ)。
pub(super) fn apply_results(state: &mut State, entry: Entry) -> Option<usize> {
    let path = normalize_path_input(&entry.path)?;
    let (dir, prefix) = split_dir_prefix(&path);
    let mut select = None;
    let listing: Option<Vec<String>> = if let Some(cycle) = &state.path_cycle {
        select = cycle.index.map(|i| i + 1);
        Some(cycle.matches.clone())
    } else if state.path_browse
        && let Some(list) = state.path_list.as_ref().filter(|l| same_dir(&l.dir, dir))
    {
        Some(
            dirs_matching(&list.names, prefix)
                .into_iter()
                .cloned()
                .collect(),
        )
    } else {
        None
    };
    state.highlight_term.clear();
    match listing {
        Some(names) => {
            if names.is_empty() {
                state.empty_message = Some("No subfolders".to_string());
            }
            let dir = if let Some(cycle) = &state.path_cycle {
                cycle.dir.clone()
            } else {
                dir.to_string()
            };
            state.results = std::iter::once(entry)
                .chain(names.iter().map(|name| dir_entry(&dir, name)))
                .collect();
        }
        None => state.results.insert(0, entry),
    }
    select
}

/// 閲覧中で、入力のディレクトリがまだ列挙されていなければ別スレッドで列挙を始める。
pub(super) fn ensure_list(state: &RefCell<State>, query: &str) {
    let Some(path) = normalize_path_input(query) else {
        return;
    };
    let (dir, _) = split_dir_prefix(&path);
    let (window, id) = {
        let mut state = state.borrow_mut();
        if !state.path_browse
            || state
                .path_list
                .as_ref()
                .is_some_and(|l| same_dir(&l.dir, dir))
            || state
                .path_list_requested
                .as_deref()
                .is_some_and(|d| same_dir(d, dir))
        {
            return;
        }
        state.path_list_reply_id = state.path_list_reply_id.wrapping_add(1);
        state.path_list_requested = Some(dir.to_string());
        (state.window, state.path_list_reply_id)
    };
    let Some(window) = window else {
        return;
    };
    LATEST_ID.store(id, Ordering::SeqCst);
    let notify = window.0 as isize;
    let dir = dir.to_string();
    std::thread::spawn(move || {
        let names = list_dirs(&dir);
        if LATEST_ID.load(Ordering::SeqCst) != id {
            return;
        }
        if let Ok(mut slot) = REPLY.lock() {
            *slot = Some((id, dir, names));
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(notify as *mut _)),
                WM_QUICK_LAUNCH_PATH_LIST,
                WPARAM(id as usize),
                LPARAM(0),
            );
        }
    });
}

/// 直下のサブディレクトリ名を並べ替えて返す。読めなければ空。
fn list_dirs(dir: &str) -> Vec<String> {
    use std::os::windows::fs::MetadataExt;
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        // `DirEntry::metadata` は列挙で得た情報で、追加の stat を伴わない
        .filter(|e| {
            e.metadata()
                .is_ok_and(|m| m.is_dir() && m.file_attributes() & ATTRIBUTE_SYSTEM == 0)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.truncate(MAX_DIRS);
    names
}

/// 列挙スレッドからの通知。古い要求の応答は捨てる。
pub(super) fn handle_list_reply(id: u32) {
    let Some((reply_id, dir, names)) = REPLY.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    if reply_id != id {
        return;
    }
    let pending = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if id != state.path_list_reply_id {
            return None;
        }
        state.path_list = Some(PathList { dir, names });
        state.path_list_requested = None;
        Some(state.path_tab_pending.take())
    });
    let Some(pending) = pending else {
        return;
    };
    let query = STATE
        .with(|state| state.borrow().edit)
        .map(read_text)
        .unwrap_or_default();
    // Tab を待たせていた入力がそのままなら補完を続け、変わっていたら一覧だけ更新する
    if pending.is_some_and(|text| normalize_path_input(&query).is_some_and(|p| p == text)) {
        tab(false);
    } else {
        STATE.with(update_results);
    }
}

/// `Tab` / `Shift+Tab`。常に押下を消費する (他のキー処理へ流さない)。
pub(super) fn on_tab() {
    tab(unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0);
}

fn tab(shift: bool) {
    let Some(edit) = STATE.with(|state| state.borrow().edit) else {
        return;
    };
    let query = read_text(edit);
    if let Some(path) = normalize_path_input(&query) {
        let text = STATE.with(|state| plan_path_tab(&mut state.borrow_mut(), &path, shift));
        match text {
            Some(text) if text != query => set_query(edit, &text),
            // 文字列は変わらないが一覧は作り直す (巡回の開始・一覧表示)
            Some(_) => STATE.with(update_results),
            None => {}
        }
        STATE.with(|state| ensure_list(state, &read_text(edit)));
    } else if !shift {
        descend_into_selected(edit, &query);
    }
}

/// パス入力中の Tab。検索欄に書き込む新しい文字列を返す (不要なら `None`)。
pub(super) fn plan_path_tab(state: &mut State, path: &str, shift: bool) -> Option<String> {
    if let Some(cycle) = state.path_cycle.as_mut().filter(|c| c.text == path) {
        let len = cycle.matches.len();
        let index = match (cycle.index, shift) {
            (None, false) => 0,
            (None, true) => len - 1,
            (Some(i), false) => (i + 1) % len,
            (Some(i), true) => (i + len - 1) % len,
        };
        cycle.index = Some(index);
        cycle.text = format!("{}{}", cycle.dir, cycle.matches[index]);
        return Some(cycle.text.clone());
    }
    state.path_cycle = None;
    state.path_browse = true;
    if shift {
        // 巡回中でなければ 1 階層上へ戻る。ルートでは何もしない
        return parent_path(path);
    }
    let (dir, prefix) = split_dir_prefix(path);
    let Some(list) = state.path_list.as_ref().filter(|l| same_dir(&l.dir, dir)) else {
        // 列挙が届いたらこの入力へ Tab を適用する
        state.path_tab_pending = Some(path.to_string());
        return None;
    };
    let hits = dirs_matching(&list.names, prefix);
    match hits.len() {
        0 => None,
        1 => Some(format!("{dir}{}\\", hits[0])),
        _ => {
            let common = common_prefix(&hits);
            let matches: Vec<String> = hits.into_iter().cloned().collect();
            let (index, text) = if prefix.is_empty() {
                // 末尾 `\` の Tab は直下の一覧を出すだけ
                (None, path.to_string())
            } else if common.chars().count() > prefix.chars().count() {
                (None, format!("{dir}{common}"))
            } else {
                (Some(0), format!("{dir}{}", matches[0]))
            };
            state.path_cycle = Some(PathCycle {
                dir: dir.to_string(),
                matches,
                index,
                text: text.clone(),
            });
            Some(text)
        }
    }
}

/// 選択中のフォルダ候補の配下へ潜る (FR-9.26)。対象は実パスの `OpenFolder` だけで、
/// プレフィックスモード (`ps ` 等) では検索欄の置換でモードが外れるため何もしない。
fn descend_into_selected(edit: HWND, query: &str) {
    if prefix_badge(query).is_some() {
        return;
    }
    let target = STATE.with(|state| {
        let state = state.borrow();
        let entry = current_selection(state.list).and_then(|row| entry_at_row(&state, row))?;
        matches!(
            entry.action,
            Action::OpenFolder(OpenMode::NewWindow | OpenMode::Reuse)
        )
        .then_some(entry.path)
    });
    let Some(path) = target.and_then(|p| normalize_path_input(&p)) else {
        return;
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.path_browse = true;
        state.path_cycle = None;
    });
    set_query(edit, &format!("{}\\", path.trim_end_matches('\\')));
}

/// 検索欄を置き換えてカーソルを末尾へ置く。`SetWindowTextW` は同期で
/// `EN_CHANGE` を送るので `update_results` が走る。STATE の借用外で呼ぶこと。
fn set_query(edit: HWND, text: &str) {
    let cursor = text.encode_utf16().count();
    unsafe {
        let _ = SetWindowTextW(edit, &HSTRING::from(text));
        let _ = SetFocus(Some(edit));
        let _ = SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(cursor)),
            Some(LPARAM(cursor as isize)),
        );
    }
}
