//! Claude Code / Codex CLI の過去セッションの読み取り (`cs ` プレフィックス、FR-9.15.6)。
//!
//! 各 CLI が自分で保存しているセッション記録を読むだけで、索引は持たない。
//! 記録は 1 件で数十 MB に達するため全体は読まず、タイトルと作業フォルダが
//! 載る先頭・末尾だけを読む。読み取りは `refresh_async` の専用スレッドで行い、
//! キー入力経路 (`latest`) は直近の結果を引くだけにする。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// セッション記録の先頭・末尾それぞれから読む量。
///
/// Claude Code はタイトル (`ai-title` / `custom-title`) を会話の途中で
/// 繰り返し追記するため、大半は末尾に最新のものがある。`/rename` を
/// 序盤で 1 回だけ使ったセッションは先頭側にしか無いので両端を読む。
const CHUNK: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub agent: Agent,
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub modified: SystemTime,
}

/// 1 ファイルから読み取った内容。Codex はタイトルを別ファイルから引くため `None`。
#[derive(Debug, Clone)]
struct Parsed {
    title: Option<String>,
    cwd: String,
}

/// ファイルごとの更新日時と読み取り結果。
type ParsedCache = HashMap<PathBuf, (SystemTime, Option<Parsed>)>;

/// 更新日時が変わっていないファイルを読み直さないためのキャッシュ。
/// 触るのは `refresh_async` のスレッドだけ (`RUNNING` で 1 本に絞っている)。
static PARSED: Mutex<Option<ParsedCache>> = Mutex::new(None);
static LATEST: Mutex<Option<Arc<Vec<Session>>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// 直近の読み取り結果 (新しい順)。まだ一度も読み終えていなければ空。
pub fn latest() -> Arc<Vec<Session>> {
    LATEST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// セッション一覧の読み直しを専用スレッドで始める。実行中なら何もしない。
pub fn refresh_async() {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let sessions = std::panic::catch_unwind(scan).ok();
        if let Some(sessions) = sessions {
            *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(sessions));
        }
        RUNNING.store(false, Ordering::Release);
    });
}

fn scan() -> Vec<Session> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut guard = PARSED.lock().unwrap_or_else(|e| e.into_inner());
    let previous = guard.take().unwrap_or_default();
    let mut next = HashMap::new();
    let mut sessions = Vec::new();

    for (path, modified) in claude_session_files(&home.join(".claude").join("projects")) {
        if let Some(parsed) = parse_cached(&path, modified, &previous, &mut next, |path| {
            let (head, tail) = read_ends(path)?;
            parse_claude(&head, &tail)
        }) && let (Some(title), Some(id)) = (parsed.title, file_stem(&path))
        {
            sessions.push(Session {
                agent: Agent::ClaudeCode,
                id,
                title,
                cwd: parsed.cwd,
                modified,
            });
        }
    }

    let codex = home.join(".codex");
    let titles = std::fs::read_to_string(codex.join("session_index.jsonl"))
        .map(|text| parse_codex_index(&text))
        .unwrap_or_default();
    for (path, modified) in codex_rollout_files(&codex.join("sessions")) {
        let Some(id) = file_stem(&path).and_then(|stem| codex_rollout_id(&stem)) else {
            continue;
        };
        let Some(title) = titles.get(&id) else {
            continue;
        };
        if let Some(parsed) = parse_cached(&path, modified, &previous, &mut next, |path| {
            let mut head = Vec::new();
            File::open(path)
                .ok()?
                .take(CHUNK)
                .read_to_end(&mut head)
                .ok()?;
            let head = String::from_utf8_lossy(&head);
            Some(Parsed {
                title: None,
                cwd: json_string_after(&head, "\"cwd\":\"")?,
            })
        }) {
            sessions.push(Session {
                agent: Agent::Codex,
                id,
                title: title.clone(),
                cwd: parsed.cwd,
                modified,
            });
        }
    }

    *guard = Some(next);
    sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
    sessions
}

/// 更新日時が前回と同じなら前回の結果を使い、変わっていれば `parse` で読み直す。
/// 更新日時は列挙時に得たものを受け取る (ファイルごとの stat を避ける)。
fn parse_cached(
    path: &Path,
    modified: SystemTime,
    previous: &ParsedCache,
    next: &mut ParsedCache,
    parse: impl FnOnce(&Path) -> Option<Parsed>,
) -> Option<Parsed> {
    let parsed = match previous.get(path) {
        Some((cached, parsed)) if *cached == modified => parsed.clone(),
        _ => parse(path),
    };
    next.insert(path.to_path_buf(), (modified, parsed.clone()));
    parsed
}

/// `projects/<プロジェクト>/<id>.jsonl` と更新日時。サブエージェントの記録
/// (`<id>/subagents/…`) は再開の対象ではないので 1 階層目だけを見る。
fn claude_session_files(projects: &Path) -> Vec<(PathBuf, SystemTime)> {
    read_dir_entries(projects)
        .into_iter()
        .filter(|entry| entry.is_dir)
        .flat_map(|project| read_dir_entries(&project.path))
        .filter(|entry| entry.path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|entry| Some((entry.path, entry.modified?)))
        .collect()
}

/// `sessions/<年>/<月>/<日>/rollout-*.jsonl` と更新日時。
fn codex_rollout_files(sessions: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut dirs = vec![sessions.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in read_dir_entries(&dir) {
            if entry.is_dir {
                dirs.push(entry.path);
            } else if entry.path.extension().is_some_and(|ext| ext == "jsonl")
                && let Some(modified) = entry.modified
            {
                files.push((entry.path, modified));
            }
        }
    }
    files
}

struct DirItem {
    path: PathBuf,
    is_dir: bool,
    modified: Option<SystemTime>,
}

/// ディレクトリの中身を、種別と更新日時つきで返す。
///
/// Windows の `DirEntry::metadata` は列挙で得た情報をそのまま返すので、
/// 1 件ずつ `is_dir` / `metadata` で stat し直さずに済む。シンボリックリンクだけは
/// 列挙の情報がリンク自身のものなので、リンク先を引き直す。
fn read_dir_entries(dir: &Path) -> Vec<DirItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let mut meta = entry.metadata().ok()?;
            if meta.file_type().is_symlink() {
                meta = std::fs::metadata(&path).ok()?;
            }
            Some(DirItem {
                is_dir: meta.is_dir(),
                modified: meta.modified().ok(),
                path,
            })
        })
        .collect()
}

fn file_stem(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_str()?.to_string())
}

/// `rollout-2026-06-04T20-46-48-<uuid>` の末尾 36 文字が id。
fn codex_rollout_id(stem: &str) -> Option<String> {
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    (stem.starts_with("rollout-") && id.bytes().filter(|b| *b == b'-').count() == 4)
        .then(|| id.to_string())
}

/// 先頭と末尾の `CHUNK` バイトずつを返す。小さいファイルは両方とも全体になる。
fn read_ends(path: &Path) -> Option<(String, String)> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(CHUNK).read_to_end(&mut head).ok()?;
    if len <= CHUNK {
        let text = String::from_utf8_lossy(&head).into_owned();
        return Some((text.clone(), text));
    }
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(len - CHUNK)).ok()?;
    file.read_to_end(&mut tail).ok()?;
    Some((
        String::from_utf8_lossy(&head).into_owned(),
        String::from_utf8_lossy(&tail).into_owned(),
    ))
}

/// Claude Code のセッション記録の両端から、タイトルと作業フォルダを取り出す。
///
/// タイトルは `custom-title` (利用者が付けた名前) を `ai-title` より優先し、
/// それぞれ末尾側の最新を先に見る。作業フォルダはどちらの端にも無ければ
/// 読み取り失敗 (再開には作業フォルダが要る)。
fn parse_claude(head: &str, tail: &str) -> Option<Parsed> {
    let cwd =
        json_string_last(tail, "\"cwd\":\"").or_else(|| json_string_last(head, "\"cwd\":\""))?;
    let title = ["\"customTitle\":\"", "\"aiTitle\":\""]
        .iter()
        .find_map(|key| {
            json_string_last(tail, key)
                .or_else(|| json_string_last(head, key))
                .filter(|title| !title.trim().is_empty())
        });
    Some(Parsed { title, cwd })
}

/// `session_index.jsonl` の id → `thread_name`。改名は追記されるので後勝ち。
fn parse_codex_index(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            Some((
                value.get("id")?.as_str()?.to_string(),
                value.get("thread_name")?.as_str()?.to_string(),
            ))
        })
        .filter(|(_, name)| !name.trim().is_empty())
        .collect()
}

/// `key` (`"name":"` の形) の直後に続く JSON 文字列を、最後の出現から取り出す。
///
/// 行全体を JSON として解釈しないのは、読み取り範囲の境目で行が途中から
/// 始まる・途中で切れることがあるため。本文中に同じ並びがあっても
/// JSON 文字列の中では `\"` にエスケープされているので誤一致しない。
fn json_string_last(text: &str, key: &str) -> Option<String> {
    let key = key.strip_suffix(":\"")?;
    text.rmatch_indices(key)
        .find_map(|(at, _)| json_string_field(&text[at + key.len()..]))
}

fn json_string_after(text: &str, key: &str) -> Option<String> {
    let key = key.strip_suffix(":\"")?;
    text.match_indices(key)
        .find_map(|(at, _)| json_string_field(&text[at + key.len()..]))
}

/// キーと値の間の JSON 空白を許容する。
fn json_string_field(rest: &str) -> Option<String> {
    let rest = rest.trim_start().strip_prefix(':')?;
    json_string_at(rest.trim_start().strip_prefix('"')?)
}

/// 開き引用符の直後から、対応する閉じ引用符までを JSON 文字列として復号する。
fn json_string_at(rest: &str) -> Option<String> {
    let mut escaped = false;
    let end = rest.char_indices().find_map(|(i, c)| {
        let found = !escaped && c == '"';
        escaped = !escaped && c == '\\';
        found.then_some(i)
    })?;
    serde_json::from_str(&format!("\"{}\"", &rest[..end])).ok()
}

/// 最終更新からの経過時間を短く表す (`5m ago` / `3h ago` / `2d ago`)。
pub fn age_label(modified: SystemTime, now: SystemTime) -> String {
    let minutes = now
        .duration_since(modified)
        .map(|elapsed| elapsed.as_secs() / 60)
        .unwrap_or(0);
    match minutes {
        0 => "just now".to_string(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn claude_prefers_custom_title_over_ai_title() {
        let head = r#"{"type":"user","cwd":"E:\\waypoint","sessionId":"x"}
{"type":"custom-title","customTitle":"fix","sessionId":"x"}"#;
        let tail = r#"{"type":"ai-title","aiTitle":"新しい仕様","sessionId":"x"}
{"type":"assistant","cwd":"E:\\waypoint\\sub","sessionId":"x"}"#;
        let parsed = parse_claude(head, tail).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("fix"));
        assert_eq!(parsed.cwd, r"E:\waypoint\sub");
    }

    #[test]
    fn claude_uses_latest_ai_title_and_decodes_escapes() {
        let tail = r#"{"type":"ai-title","aiTitle":"old"}
{"type":"user","cwd":"C:\\Users\\me"}
{"type":"ai-title","aiTitle":"say \"hi\" \u3042"}"#;
        let parsed = parse_claude("", tail).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("say \"hi\" あ"));
        assert_eq!(parsed.cwd, r"C:\Users\me");
    }

    #[test]
    fn claude_without_title_or_cwd() {
        assert!(parse_claude("", r#"{"type":"ai-title","aiTitle":"t"}"#).is_none());
        let parsed = parse_claude(r#"{"cwd":"E:\\a"}"#, "").unwrap();
        assert_eq!(parsed.title, None);
    }

    #[test]
    fn session_fields_allow_json_whitespace() {
        let text = r#"{ "cwd" : "E:\\work", "aiTitle": "Review" }"#;
        let parsed = parse_claude(text, "").unwrap();
        assert_eq!(parsed.cwd, r"E:\work");
        assert_eq!(parsed.title.as_deref(), Some("Review"));
        assert_eq!(
            json_string_after(text, "\"cwd\":\"").as_deref(),
            Some(r"E:\work")
        );
    }

    #[test]
    fn blank_custom_title_falls_back_to_ai_title() {
        let text = r#"{"cwd":"E:\\work","customTitle":"  ","aiTitle":"Review"}"#;
        assert_eq!(
            parse_claude(text, "").unwrap().title.as_deref(),
            Some("Review")
        );
    }

    #[test]
    fn escaped_key_inside_message_text_is_ignored() {
        let tail = r#"{"type":"user","message":"{\"aiTitle\":\"fake\"}","cwd":"E:\\a"}"#;
        assert_eq!(parse_claude("", tail).unwrap().title, None);
    }

    #[test]
    fn truncated_value_at_chunk_end_falls_back_to_earlier_match() {
        let tail = "{\"type\":\"ai-title\",\"aiTitle\":\"ok\"}\n{\"cwd\":\"E:\\\\a\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"cut";
        assert_eq!(parse_claude("", tail).unwrap().title.as_deref(), Some("ok"));
    }

    #[test]
    fn codex_index_last_name_wins() {
        let text = r#"{"id":"a","thread_name":"first","updated_at":"2026"}
{"id":"b","thread_name":"other"}
{"id":"a","thread_name":"renamed"}
broken line"#;
        let titles = parse_codex_index(text);
        assert_eq!(titles["a"], "renamed");
        assert_eq!(titles["b"], "other");
    }

    #[test]
    fn codex_rollout_id_from_file_stem() {
        assert_eq!(
            codex_rollout_id("rollout-2026-06-04T20-46-48-019e9275-04bd-7740-bcf4-850037b2ce82")
                .as_deref(),
            Some("019e9275-04bd-7740-bcf4-850037b2ce82")
        );
        assert_eq!(codex_rollout_id("notes"), None);
    }

    #[test]
    fn codex_cwd_is_first_occurrence_in_session_meta() {
        let head = r#"{"type":"session_meta","payload":{"id":"x","cwd":"E:\\st","base_instructions":{"text":"..."}}}"#;
        assert_eq!(
            json_string_after(head, "\"cwd\":\"").as_deref(),
            Some(r"E:\st")
        );
    }

    /// 実機のセッション記録で読み取り時間を測る。初回 (全件読み) と 2 回目
    /// (更新日時キャッシュ) の差を見る。
    #[test]
    #[ignore]
    fn bench_scan_real_sessions() {
        for round in ["cold", "warm"] {
            let started = std::time::Instant::now();
            let sessions = scan();
            println!(
                "{round}: {} sessions in {:?}",
                sessions.len(),
                started.elapsed()
            );
        }
        let sessions = scan();
        let codex = sessions.iter().filter(|s| s.agent == Agent::Codex).count();
        println!("codex: {codex}");
        for session in sessions.iter().filter(|s| s.agent == Agent::Codex).take(3) {
            println!("{:?} | {} | {}", session.agent, session.title, session.cwd);
        }
        for session in sessions.iter().take(8) {
            println!("{:?} | {} | {}", session.agent, session.title, session.cwd);
        }
    }

    #[test]
    fn age_labels() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86400);
        let ago = |secs| age_label(now - Duration::from_secs(secs), now);
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(5 * 60), "5m ago");
        assert_eq!(ago(3 * 3600), "3h ago");
        assert_eq!(ago(2 * 86400), "2d ago");
        assert_eq!(age_label(now + Duration::from_secs(60), now), "just now");
    }
}
