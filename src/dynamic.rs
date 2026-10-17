//! QAP の In the Works に相当する動的メニューを構築する。
//!
//! 列挙はメニューを閉じた後に行い、表示経路ではキャッシュだけを使う。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::shell::ComGuard;

mod open_windows;
use open_windows::enumerate_windows;
pub(crate) use open_windows::window_titles_by_pid;

const ITEM_LIMIT: usize = 10;

/// Recent の走査を一つに直列化する。多重に COM 解決を走らせない。
static REFRESHING: AtomicBool = AtomicBool::new(false);
/// ウィンドウ列挙の多重起動を避ける。Recent の走査とは独立に回す。
static ENUMERATING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEntry {
    pub title: String,
    pub hwnd: isize,
    /// 所有プロセスの実行ファイル名 (例: `chrome.exe`)。取得できなければ空。
    /// タイトルにアプリ名が出ないウィンドウも `w chrome` のように
    /// プロセス名で検索できるようにするために持つ。
    pub process_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menus {
    pub recent_folders: Vec<PathEntry>,
    pub recent_files: Vec<PathEntry>,
    pub frequent_folders: Vec<PathEntry>,
    pub frequent_files: Vec<PathEntry>,
    /// トレイの "Current Windows" サブメニュー用。表示件数を絞るため
    /// `ITEM_LIMIT` で切り詰めてある。全件が要る場合は `all_windows` を使う
    pub current_windows: Vec<WindowEntry>,
    /// Quick Launch (`w ` プレフィックス) の検索索引用。こちらは表示件数の
    /// 制限を持たない。`current_windows` を Quick Launch の索引構築に
    /// 使い回すと、開いているウィンドウが `ITEM_LIMIT` (10) を超えた分は
    /// 検索してもヒットしなくなる (実際に報告された不具合)
    pub all_windows: Vec<WindowEntry>,
}

#[derive(Debug, Clone)]
struct RecentItem {
    entry: PathEntry,
    is_dir: bool,
    modified: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    entries: BTreeMap<String, HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    name: String,
    path: String,
    is_dir: bool,
    count: u64,
    last_seen: u64,
    last_used: u64,
}

/// 列挙をバックグラウンドスレッドで実行する。
///
/// Recent フォルダの `.lnk` を 1 件ずつ COM (`IPersistFile`) で解決する
/// コストが実測で数十 ms あり、UI スレッド (`wnd_proc`) 上でメニューを
/// 閉じるたびに同期実行すると次のトリガーへの応答が詰まる。
/// COM は STA なので、このスレッド自身で `CoInitializeEx` してから使う。
///
/// 開いているウィンドウ (`w ` の一覧) は別スレッドで先に届ける。Recent の
/// 走査は実体の存在確認で不達のネットワークパスに当たると長く止まるため、
/// 同じスレッドに載せると窓一覧の更新まで道連れで遅れる。
///
/// Recent 側が実行中なら何もしない (`false` を返す)。結果は `notify` へ
/// `message` で、各段が終わるたびに通知する。呼び出し側は `take_refreshed()` で
/// 取り出す (どの時点でも窓一覧と Recent の両方を含む一貫した値になる)。
pub fn refresh_async(notify: HWND, message: u32) -> bool {
    let notify = notify.0 as isize;
    if !ENUMERATING.swap(true, Ordering::AcqRel) {
        thread::spawn(move || {
            let all_windows = enumerate_windows();
            publish(notify, message, |menus| set_windows(menus, all_windows));
            // 通知の後に下ろす。先に下ろすと次の列挙の結果が追い越される
            ENUMERATING.store(false, Ordering::Release);
        });
    }
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return false;
    }
    thread::spawn(move || {
        let started = crate::startup_timing::enabled().then(std::time::Instant::now);
        let _guard = ComGuard::new();
        let recent = refresh_recent();
        if let Some(started) = started {
            crate::startup_timing::mark_elapsed(
                "background dynamic refresh finished",
                started.elapsed(),
            );
        }
        publish(notify, message, |menus| recent.apply_to(menus));
        REFRESHING.store(false, Ordering::Release);
    });
    true
}

/// `refresh_async` が置いた結果を取り出す。通知を受けたときに一度だけ呼ぶ。
pub fn take_refreshed() -> Option<Menus> {
    RESULT.with_lock(std::mem::take)
}

struct ResultSlot(std::sync::Mutex<Option<Menus>>);

impl ResultSlot {
    fn with_lock<R>(&self, f: impl FnOnce(&mut Option<Menus>) -> R) -> R {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }
}

static RESULT: ResultSlot = ResultSlot(std::sync::Mutex::new(None));

/// 最新の列挙結果。窓一覧と Recent は別スレッドが自分の担当分だけ書き換える
/// ので、通知する値は常にここから複製する (片方だけの値を渡さない)。
static CURRENT: LazyLock<Mutex<Menus>> = LazyLock::new(Mutex::default);

/// `CURRENT` を更新し、その複製を `RESULT` に置いて `notify` へ知らせる。
fn publish(notify: isize, message: u32, update: impl FnOnce(&mut Menus)) {
    {
        // RESULT への書き込みも CURRENT のロック内で行い、古い複製が
        // 新しい複製を追い越して置かれないようにする
        let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        update(&mut current);
        let snapshot = current.clone();
        RESULT.with_lock(|slot| *slot = Some(snapshot));
    }
    unsafe {
        let _ = PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
    }
}

/// Recent / Frequent 側の列挙結果。
struct RecentPart {
    recent_folders: Vec<PathEntry>,
    recent_files: Vec<PathEntry>,
    frequent_folders: Vec<PathEntry>,
    frequent_files: Vec<PathEntry>,
}

impl RecentPart {
    fn apply_to(self, menus: &mut Menus) {
        menus.recent_folders = self.recent_folders;
        menus.recent_files = self.recent_files;
        menus.frequent_folders = self.frequent_folders;
        menus.frequent_files = self.frequent_files;
    }
}

fn set_windows(menus: &mut Menus, all_windows: Vec<WindowEntry>) {
    menus.current_windows = all_windows.iter().take(ITEM_LIMIT).cloned().collect();
    menus.all_windows = all_windows;
}

/// Windows の状態を読み、次回表示用のスナップショットを作る。
pub fn refresh() -> Menus {
    let mut menus = Menus::default();
    refresh_recent().apply_to(&mut menus);
    set_windows(&mut menus, enumerate_windows());
    menus
}

fn refresh_recent() -> RecentPart {
    let recent = scan_recent_items();
    let mut history = load_history();
    // 何も変わっていないのに書くと、原子的保存だけで 12〜15ms かかる
    // (実測)。Quick Launch の表示と終了のたびに走るため、変化時だけ保存する
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let updated = update_history(&mut history, &recent);
    let pruned = prune_history(&mut history, &recent, now);
    if updated || pruned {
        let _ = save_history(&history);
    }

    RecentPart {
        recent_folders: recent_entries(&recent, true),
        recent_files: recent_entries(&recent, false),
        frequent_folders: frequent_entries(&history, true),
        frequent_files: frequent_entries(&history, false),
    }
}

fn recent_entries(items: &[RecentItem], is_dir: bool) -> Vec<PathEntry> {
    items
        .iter()
        .filter(|item| item.is_dir == is_dir)
        .take(ITEM_LIMIT)
        .map(|item| item.entry.clone())
        .collect()
}

fn frequent_entries(history: &History, is_dir: bool) -> Vec<PathEntry> {
    let mut entries: Vec<&HistoryEntry> = history
        .entries
        .values()
        .filter(|entry| entry.is_dir == is_dir)
        .collect();
    entries.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.count),
            std::cmp::Reverse(entry.last_used),
        )
    });
    // 履歴は剪定されず増える一方で、存在確認は不達のネットワークパスで固まる。
    // 並べ替えてから、表示する分 (ITEM_LIMIT) が揃うまでだけ確認する
    entries
        .into_iter()
        .filter(|entry| Path::new(&entry.path).exists())
        .take(ITEM_LIMIT)
        .map(|entry| PathEntry {
            name: entry.name.clone(),
            path: entry.path.clone(),
        })
        .collect()
}

fn scan_recent_items() -> Vec<RecentItem> {
    let Some(dir) = dirs::data_dir().map(|path| path.join("Microsoft\\Windows\\Recent")) else {
        return Vec::new();
    };
    let Ok(files) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for file in files.flatten() {
        let link = file.path();
        if !link
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            continue;
        }
        // 更新日時は列挙で得た値を使う (リンクごとの stat をもう一度払わない)。
        // 取れないリンクは解決しない (これまでも `resolve_target_cached` が弾いていた)
        let Some(modified_time) = file.metadata().and_then(|m| m.modified()).ok() else {
            continue;
        };
        let Some(path) = crate::apps::resolve_target_at(&link.to_string_lossy(), modified_time)
        else {
            continue;
        };
        let target = PathBuf::from(&path);
        // is_dir と is_file を別々に呼ぶと stat が 2 回になる
        let Ok(kind) = fs::metadata(&target) else {
            continue;
        };
        let is_dir = kind.is_dir();
        if !is_dir && !kind.is_file() {
            continue;
        }
        let key = path.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let modified = modified_time
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        items.push(RecentItem {
            entry: PathEntry {
                name: display_name(&target),
                path,
            },
            is_dir,
            modified,
        });
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.modified));
    items
}

/// 履歴を更新し、保存すべき変化があったかを返す。
fn update_history(history: &mut History, recent: &[RecentItem]) -> bool {
    let mut changed = false;
    for item in recent {
        let key = item.entry.path.to_lowercase();
        let entry = history.entries.entry(key).or_insert_with(|| HistoryEntry {
            name: item.entry.name.clone(),
            path: item.entry.path.clone(),
            is_dir: item.is_dir,
            count: 0,
            last_seen: 0,
            last_used: 0,
        });
        changed |= entry.name != item.entry.name
            || entry.path != item.entry.path
            || entry.is_dir != item.is_dir
            || item.modified > entry.last_seen;
        entry.name.clone_from(&item.entry.name);
        entry.path.clone_from(&item.entry.path);
        entry.is_dir = item.is_dir;
        if item.modified > entry.last_seen {
            entry.count += 1;
            entry.last_seen = item.modified;
            entry.last_used = item.modified;
        }
    }
    changed
}

/// 最終利用からこの期間を過ぎた履歴は落とす。
const HISTORY_MAX_AGE_SECS: u64 = 365 * 24 * 60 * 60;
/// 履歴の件数上限。超えた分は最終利用が古いものから落とす。
const HISTORY_MAX_ENTRIES: usize = 2000;

/// 古い履歴と上限を超えた履歴を取り除き、変化があったかを返す。
///
/// 履歴は無剪定だと増える一方で、毎回の読み込み・保存・存在確認の対象になる。
/// 今の Recent に載っている項目は落とさない。落としても次の更新で作り直され、
/// 保存が毎回走ってしまうため。
fn prune_history(history: &mut History, recent: &[RecentItem], now: u64) -> bool {
    let current: HashSet<String> = recent
        .iter()
        .map(|item| item.entry.path.to_lowercase())
        .collect();
    let before = history.entries.len();
    history.entries.retain(|key, entry| {
        current.contains(key) || now.saturating_sub(entry.last_used) <= HISTORY_MAX_AGE_SECS
    });
    let excess = history.entries.len().saturating_sub(HISTORY_MAX_ENTRIES);
    if excess > 0 {
        let mut by_age: Vec<(u64, String)> = history
            .entries
            .iter()
            .filter(|(key, _)| !current.contains(*key))
            .map(|(key, entry)| (entry.last_used, key.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, key) in by_age.into_iter().take(excess) {
            history.entries.remove(&key);
        }
    }
    history.entries.len() != before
}

fn history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("waypoint").join("history.json"))
}

fn load_history() -> History {
    let Some(path) = history_path() else {
        return History::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_history(history: &History) -> std::io::Result<()> {
    let Some(path) = history_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(history).map_err(std::io::Error::other)?;
    crate::config::write_atomic(&path, &text)
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_only_counts_a_new_recent_timestamp() {
        let item = RecentItem {
            entry: PathEntry {
                name: "file.txt".into(),
                path: "C:\\file.txt".into(),
            },
            is_dir: false,
            modified: 100,
        };
        let mut history = History::default();
        update_history(&mut history, std::slice::from_ref(&item));
        update_history(&mut history, std::slice::from_ref(&item));
        assert_eq!(history.entries["c:\\file.txt"].count, 1);
    }

    fn history_entry(last_used: u64) -> HistoryEntry {
        HistoryEntry {
            name: "x".into(),
            path: "x".into(),
            is_dir: true,
            count: 1,
            last_seen: last_used,
            last_used,
        }
    }

    #[test]
    fn prune_history_drops_stale_and_excess_but_keeps_current_recent_items() {
        let now = 2 * HISTORY_MAX_AGE_SECS;
        let mut history = History::default();
        history.entries.insert("stale".into(), history_entry(1));
        // 今の Recent に載っている項目は古くても残す (残さないと毎回作り直して保存する)
        history
            .entries
            .insert("current/old".into(), history_entry(1));
        for i in 0..HISTORY_MAX_ENTRIES + 5 {
            history
                .entries
                .insert(format!("fresh/{i:05}"), history_entry(now - 10 + i as u64));
        }
        let recent = [RecentItem {
            entry: PathEntry {
                name: "old".into(),
                path: "current/old".into(),
            },
            is_dir: true,
            modified: 1,
        }];
        assert!(prune_history(&mut history, &recent, now));
        assert!(!history.entries.contains_key("stale"));
        assert!(history.entries.contains_key("current/old"));
        // 上限超過ぶんは最終利用が古いもの (fresh/00000 側) から落ちる
        assert!(!history.entries.contains_key("fresh/00000"));
        let newest = format!("fresh/{:05}", HISTORY_MAX_ENTRIES + 4);
        assert!(history.entries.contains_key(&newest));
        assert_eq!(history.entries.len(), HISTORY_MAX_ENTRIES);
        // 2 回目は変化なし
        assert!(!prune_history(&mut history, &recent, now));
    }
}
