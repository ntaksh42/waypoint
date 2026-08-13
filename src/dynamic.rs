//! QAP の In the Works に相当する動的メニューを構築する。
//!
//! 列挙はメニューを閉じた後に行い、表示経路ではキャッシュだけを使う。

use std::collections::{BTreeMap, HashSet};
use std::ffi::c_void;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::{BOOL, Interface, PCWSTR};

const ITEM_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEntry {
    pub title: String,
    pub hwnd: isize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menus {
    pub recent_folders: Vec<PathEntry>,
    pub recent_files: Vec<PathEntry>,
    pub frequent_folders: Vec<PathEntry>,
    pub frequent_files: Vec<PathEntry>,
    pub current_windows: Vec<WindowEntry>,
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

/// Windows の状態を読み、次回表示用のスナップショットを作る。
pub fn refresh() -> Menus {
    let recent = scan_recent_items();
    let mut history = load_history();
    update_history(&mut history, &recent);
    let _ = save_history(&history);

    Menus {
        recent_folders: recent_entries(&recent, true),
        recent_files: recent_entries(&recent, false),
        frequent_folders: frequent_entries(&history, true),
        frequent_files: frequent_entries(&history, false),
        current_windows: enumerate_windows(),
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
        .filter(|entry| entry.is_dir == is_dir && Path::new(&entry.path).exists())
        .collect();
    entries.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.count),
            std::cmp::Reverse(entry.last_used),
        )
    });
    entries
        .into_iter()
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
        let Some(path) = resolve_shortcut(&link) else {
            continue;
        };
        let target = PathBuf::from(&path);
        let is_dir = target.is_dir();
        if !is_dir && !target.is_file() {
            continue;
        }
        let key = path.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let modified = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
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

fn resolve_shortcut(link_path: &Path) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist = link.cast::<IPersistFile>().ok()?;
        let wide: Vec<u16> = link_path.as_os_str().encode_wide().chain(Some(0)).collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut target = vec![0u16; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
        let len = target.iter().position(|ch| *ch == 0)?;
        (len > 0).then(|| String::from_utf16_lossy(&target[..len]))
    }
}

fn update_history(history: &mut History, recent: &[RecentItem]) {
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
        entry.name.clone_from(&item.entry.name);
        entry.path.clone_from(&item.entry.path);
        entry.is_dir = item.is_dir;
        if item.modified > entry.last_seen {
            entry.count += 1;
            entry.last_seen = item.modified;
            entry.last_used = item.modified;
        }
    }
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
    let text = serde_json::to_string_pretty(history).map_err(std::io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp, path)
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn enumerate_windows() -> Vec<WindowEntry> {
    let mut windows = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_window),
            LPARAM((&mut windows as *mut Vec<WindowEntry>) as isize),
        );
    }
    windows.truncate(ITEM_LIMIT);
    windows
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) <= 0 {
            return true.into();
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == GetCurrentProcessId() {
            return true.into();
        }
        let mut title = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
        let len = GetWindowTextW(hwnd, &mut title);
        if len > 0 {
            let windows = &mut *(lparam.0 as *mut c_void as *mut Vec<WindowEntry>);
            windows.push(WindowEntry {
                title: String::from_utf16_lossy(&title[..len as usize]),
                hwnd: hwnd.0 as isize,
            });
        }
        true.into()
    }
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
}
