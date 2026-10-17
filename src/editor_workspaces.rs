//! VS Code / Cursor のワークスペース履歴。外部ストアは背景で読み取るだけ。

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Editor {
    Code,
    Cursor,
}

impl Editor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Cursor => "Cursor",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Cursor => "cursor",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub editor: Editor,
    pub uri: String,
    pub path: String,
    pub is_file: bool,
    pub remote: bool,
}

static LATEST: Mutex<Option<Arc<Vec<Workspace>>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_FINISHED: Mutex<Option<Instant>> = Mutex::new(None);

pub fn latest() -> Arc<Vec<Workspace>> {
    LATEST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// 完了通知以外の処理を専用スレッドで行い、表示経路では結果を待たない。
pub fn refresh_async(notify: impl FnOnce() + Send + 'static) {
    if LAST_FINISHED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|last| last.elapsed() < Duration::from_secs(5))
        || RUNNING.swap(true, Ordering::AcqRel)
    {
        return;
    }
    std::thread::spawn(move || {
        if let Ok(workspaces) = std::panic::catch_unwind(scan) {
            *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(workspaces));
        }
        *LAST_FINISHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        RUNNING.store(false, Ordering::Release);
        notify();
    });
}

fn scan() -> Vec<Workspace> {
    let Some(appdata) = dirs::config_dir() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for editor in [Editor::Code, Editor::Cursor] {
        let user = appdata.join(editor.label()).join("User");
        let shared =
            dirs::home_dir().map(|home| home.join(".vscode-shared/sharedStorage/state.vscdb"));
        let history = if editor == Editor::Code {
            shared.as_deref().and_then(read_history)
        } else {
            None
        }
        .or_else(|| read_history(&user.join("globalStorage/state.vscdb")));
        let candidates =
            history.unwrap_or_else(|| read_workspace_storage(&user.join("workspaceStorage")));
        let mut seen = HashSet::new();
        for (uri, is_file) in candidates {
            let Some(workspace) = parse_workspace(editor, &uri, is_file) else {
                continue;
            };
            let key = if workspace.remote {
                uri.clone()
            } else {
                workspace.path.to_lowercase()
            };
            if seen.insert(key) && available(&workspace) {
                result.push(workspace);
            }
        }
    }
    result
}

fn available(workspace: &Workspace) -> bool {
    // UNC は stat が SMB のタイムアウトまで待つため確認しない。
    workspace.remote
        || workspace.path.starts_with(r"\\")
        || if workspace.is_file {
            Path::new(&workspace.path).is_file()
        } else {
            Path::new(&workspace.path).is_dir()
        }
}

fn read_history(path: &Path) -> Option<Vec<(String, bool)>> {
    let uri = format!(
        "file:{}?immutable=1",
        crate::browser_history::encode_uri_path(&path.display().to_string())
    );
    let db = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let json: String = db
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    parse_history(&json)
}

fn parse_history(json: &str) -> Option<Vec<(String, bool)>> {
    let value: Value = serde_json::from_str(json).ok()?;
    Some(
        value
            .get("entries")?
            .as_array()?
            .iter()
            .filter_map(|entry| {
                if let Some(uri) = entry.get("folderUri").and_then(Value::as_str) {
                    Some((uri.to_string(), false))
                } else {
                    entry
                        .get("workspace")?
                        .get("configPath")?
                        .as_str()
                        .map(|uri| (uri.to_string(), true))
                }
            })
            .collect(),
    )
}

fn read_workspace_storage(root: &Path) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for dir in dirs.flatten() {
        let Ok(meta) = dir.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        let Some(value) = std::fs::read(dir.path().join("workspace.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        let target = value
            .get("folder")
            .and_then(Value::as_str)
            .map(|uri| (uri, false))
            .or_else(|| {
                value
                    .get("workspace")
                    .and_then(Value::as_str)
                    .map(|uri| (uri, true))
            });
        if let Some((uri, file)) = target {
            entries.push((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                uri.to_string(),
                file,
            ));
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    entries
        .into_iter()
        .map(|(_, uri, file)| (uri, file))
        .collect()
}

fn parse_workspace(editor: Editor, uri: &str, is_file: bool) -> Option<Workspace> {
    let remote = uri.starts_with("vscode-remote://");
    let path = if remote {
        let rest = uri.strip_prefix("vscode-remote://")?;
        let (authority, path) = rest.split_once('/')?;
        if authority.is_empty() || path.is_empty() {
            return None;
        }
        uri.to_string()
    } else {
        local_path(uri)?
    };
    Some(Workspace {
        editor,
        uri: uri.to_string(),
        path,
        is_file,
        remote,
    })
}

/// file URI のパスだけを復号し、Windows のドライブ・UNC 表記へ変換する。
fn local_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let (authority, path) = rest.split_once('/')?;
    let path = decode(path.split(['?', '#']).next()?)?;
    if path.contains('\0') {
        return None;
    }
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return Some(format!(
            r"\\{}\{}",
            decode(authority)?,
            path.replace('/', r"\")
        ));
    }
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'/' {
        return None;
    }
    Some(format!(
        "{}{}",
        path[..1].to_ascii_uppercase(),
        path[1..].replace('/', r"\")
    ))
}

fn decode(value: &str) -> Option<String> {
    let mut result = Vec::new();
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        result.push(if byte == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else {
            byte
        });
    }
    String::from_utf8(result).ok()
}

/// 並び順を保持した候補。読み取り順が最近順なので使用頻度では並べ替えない。
pub(crate) fn entries(workspaces: &[Workspace]) -> Vec<crate::quick_launch::Entry> {
    workspaces
        .iter()
        .map(|workspace| {
            let name = workspace
                .path
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&workspace.path);
            crate::quick_launch::Entry {
                azure: None,
                name: (if workspace.remote {
                    decode(name).unwrap_or_else(|| name.to_string())
                } else {
                    name.to_string()
                })
                .trim_end_matches(".code-workspace")
                .to_string(),
                breadcrumb: format!(
                    "{} · {}{}",
                    workspace.path,
                    workspace.editor.label(),
                    if workspace.remote { " · Remote" } else { "" }
                ),
                path: workspace.path.clone(),
                action: crate::quick_launch::Action::OpenEditorWorkspace(
                    workspace.editor,
                    workspace.uri.clone(),
                    workspace.is_file,
                ),
                branch: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
