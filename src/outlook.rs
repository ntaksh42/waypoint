//! CLI for Microsoft 365 を介した Outlook メール検索。

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;

use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub subject: String,
    pub sender: String,
    pub received: String,
    pub web_link: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchReply {
    pub messages: Vec<Message>,
    pub error: Option<String>,
}

/// `m365 request` をワーカースレッドで実行する。CLI の標準出力だけを JSON として
/// 扱い、認証トークンやエラー出力は waypoint の設定・ログへ残さない。
pub fn search_async(query: String, request_id: u32, notify: HWND, message: u32) {
    let notify = notify.0 as isize;
    thread::spawn(move || {
        let reply = search(&query);
        let mut pending = pending_replies()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.insert(request_id, reply);
        pending.retain(|id, _| *id >= request_id.saturating_sub(3));
        drop(pending);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(notify as *mut _)),
                message,
                WPARAM(request_id as usize),
                LPARAM(0),
            );
        }
    });
}

pub fn take_search_reply(request_id: u32) -> Option<SearchReply> {
    pending_replies()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&request_id)
}

fn pending_replies() -> &'static Mutex<HashMap<u32, SearchReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, SearchReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn search(query: &str) -> SearchReply {
    let body = search_body(query).to_string();
    let output = match Command::new("m365")
        .args([
            "request",
            "--method",
            "post",
            "--url",
            "@graph/search/query",
            "--body",
            &body,
            "--output",
            "json",
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SearchReply {
                messages: Vec::new(),
                error: Some("CLI for Microsoft 365 (m365) is not installed.".to_string()),
            };
        }
        Err(_) => {
            return SearchReply {
                messages: Vec::new(),
                error: Some("Could not start CLI for Microsoft 365.".to_string()),
            };
        }
    };
    if !output.status.success() {
        return SearchReply {
            messages: Vec::new(),
            error: Some(
                "Outlook search failed. Sign in with m365 login and grant Mail.Read.".to_string(),
            ),
        };
    }
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(value) => SearchReply {
            messages: parse_messages(&value),
            error: None,
        },
        Err(_) => SearchReply {
            messages: Vec::new(),
            error: Some("CLI for Microsoft 365 returned invalid search data.".to_string()),
        },
    }
}

fn search_body(query: &str) -> Value {
    json!({
        "requests": [{
            "entityTypes": ["message"],
            "query": { "queryString": query },
            "from": 0,
            "size": 24,
            "fields": ["subject", "from", "receivedDateTime", "webLink"]
        }]
    })
}

pub fn parse_messages(value: &Value) -> Vec<Message> {
    value
        .pointer("/value/0/hitsContainers/0/hits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            let resource = hit.get("resource")?;
            let web_link = resource.get("webLink")?.as_str()?.to_string();
            Some(Message {
                subject: resource
                    .get("subject")
                    .and_then(Value::as_str)
                    .filter(|subject| !subject.is_empty())
                    .unwrap_or("(No subject)")
                    .to_string(),
                sender: resource
                    .pointer("/from/emailAddress/name")
                    .or_else(|| resource.pointer("/from/emailAddress/address"))
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown sender")
                    .to_string(),
                received: resource
                    .get("receivedDateTime")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                web_link,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_messages, search_body};
    use serde_json::json;

    #[test]
    fn search_body_limits_results_and_requests_message_fields() {
        let body = search_body("roadmap");
        assert_eq!(body["requests"][0]["size"], 24);
        assert_eq!(body["requests"][0]["query"]["queryString"], "roadmap");
        assert_eq!(body["requests"][0]["entityTypes"][0], "message");
    }

    #[test]
    fn parse_messages_uses_only_openable_hits() {
        let value = json!({ "value": [{ "hitsContainers": [{ "hits": [
            { "resource": { "subject": "Roadmap", "from": { "emailAddress": { "name": "Ada" } }, "receivedDateTime": "2026-09-18T00:00:00Z", "webLink": "https://outlook.office.com/mail/id" } },
            { "resource": { "subject": "Missing link" } }
        ] }] }] });
        let messages = parse_messages(&value);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].subject, "Roadmap");
        assert_eq!(messages[0].sender, "Ada");
    }
}
