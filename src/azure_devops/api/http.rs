//! Azure DevOps REST API への HTTP リクエスト基盤 (リトライ・認証)。

use std::io::Read;
use std::thread;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub(crate) const API_VERSION: &str = "7.1";
const REQUEST_RETRIES: usize = 2;
const RETRY_DELAY: Duration = Duration::from_millis(350);
/// Azure DevOps の API 応答として十分な余裕を持たせつつ、壊れた中継や
/// 想定外の応答で常駐プロセスのメモリを使い切らないための上限。
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn get_json(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
) -> Result<Value, String> {
    let mut last_error = None;
    for attempt in 0..=REQUEST_RETRIES {
        let mut delay = response_retry_delay(&reqwest::header::HeaderMap::new(), attempt);
        match client
            .get(url)
            .header("Authorization", authorization(pat))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                let retry_after = retry_after_delay(response.headers());
                let value = response_json(response)?;
                if let Some(delay) = retry_after {
                    thread::sleep(delay);
                }
                return Ok(value);
            }
            Ok(response) => {
                let status = response.status();
                delay = response_retry_delay(response.headers(), attempt);
                last_error = Some(format!("Azure DevOps request returned HTTP {status}"));
                if !retryable_status(status.as_u16()) || attempt == REQUEST_RETRIES {
                    break;
                }
            }
            Err(error) => {
                last_error = Some(format!("Azure DevOps request failed: {error}"));
                if attempt == REQUEST_RETRIES {
                    break;
                }
            }
        }
        thread::sleep(delay);
    }
    Err(last_error.unwrap_or_else(|| "Azure DevOps request failed.".to_string()))
}

pub(crate) fn post_json(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    body: &Value,
) -> Result<Value, String> {
    let mut last_error = None;
    for attempt in 0..=REQUEST_RETRIES {
        let mut delay = response_retry_delay(&reqwest::header::HeaderMap::new(), attempt);
        match client
            .post(url)
            .header("Authorization", authorization(pat))
            .json(body)
            .send()
        {
            Ok(response) if response.status().is_success() => {
                let retry_after = retry_after_delay(response.headers());
                let value = response_json(response)?;
                if let Some(delay) = retry_after {
                    thread::sleep(delay);
                }
                return Ok(value);
            }
            Ok(response) => {
                let status = response.status();
                delay = response_retry_delay(response.headers(), attempt);
                last_error = Some(format!("Azure DevOps request returned HTTP {status}"));
                if !retryable_status(status.as_u16()) || attempt == REQUEST_RETRIES {
                    break;
                }
            }
            Err(error) => {
                last_error = Some(format!("Azure DevOps request failed: {error}"));
                if attempt == REQUEST_RETRIES {
                    break;
                }
            }
        }
        thread::sleep(delay);
    }
    Err(last_error.unwrap_or_else(|| "Azure DevOps request failed.".to_string()))
}

fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn response_retry_delay(headers: &reqwest::header::HeaderMap, attempt: usize) -> Duration {
    retry_after_delay(headers).unwrap_or(RETRY_DELAY * (attempt as u32 + 1))
}

pub(crate) fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(35))
        // PAT 付き要求を別 URL へ自動追従させない。Azure DevOps REST API は
        // 通常の取得でリダイレクトを必要としない。
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not initialize Azure DevOps client: {error}"))
}

/// 応答ボディを上限付きで読み、JSON として解釈する。
fn response_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("Azure DevOps response exceeded the size limit.".to_string());
    }

    let mut bytes = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Failed to read Azure DevOps response: {error}"))?;
    parse_response_bytes(&bytes)
}

fn parse_response_bytes(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Azure DevOps response exceeded the size limit.".to_string());
    }
    serde_json::from_slice(bytes)
        .map_err(|error| format!("Azure DevOps response was invalid: {error}"))
}

fn authorization(pat: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!(":{pat}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_http_statuses_are_retried() {
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }

    #[test]
    fn retry_after_header_overrides_the_local_backoff() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "3".parse().unwrap());
        assert_eq!(response_retry_delay(&headers, 0), Duration::from_secs(3));
    }

    #[test]
    fn missing_retry_after_uses_incremental_local_backoff() {
        assert_eq!(
            response_retry_delay(&reqwest::header::HeaderMap::new(), 1),
            Duration::from_millis(700)
        );
    }

    #[test]
    fn oversized_response_body_is_rejected_before_json_parsing() {
        assert!(parse_response_bytes(&vec![b' '; MAX_RESPONSE_BYTES + 1]).is_err());
    }
}
