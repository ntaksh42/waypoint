//! Azure DevOps REST API への HTTP リクエスト基盤 (リトライ・認証)。

use std::thread;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub(crate) const API_VERSION: &str = "7.1";
const REQUEST_RETRIES: usize = 2;
const RETRY_DELAY: Duration = Duration::from_millis(350);

pub(crate) fn get_json(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
) -> Result<Value, String> {
    let mut last_error = None;
    for attempt in 0..=REQUEST_RETRIES {
        match client
            .get(url)
            .header("Authorization", authorization(pat))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                return response
                    .json()
                    .map_err(|error| format!("Azure DevOps response was invalid: {error}"));
            }
            Ok(response) => {
                let status = response.status();
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
        thread::sleep(RETRY_DELAY * (attempt as u32 + 1));
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
        match client
            .post(url)
            .header("Authorization", authorization(pat))
            .json(body)
            .send()
        {
            Ok(response) if response.status().is_success() => {
                return response
                    .json()
                    .map_err(|error| format!("Azure DevOps response was invalid: {error}"));
            }
            Ok(response) => {
                let status = response.status();
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
        thread::sleep(RETRY_DELAY * (attempt as u32 + 1));
    }
    Err(last_error.unwrap_or_else(|| "Azure DevOps request failed.".to_string()))
}

pub(crate) fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

pub(crate) fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("Could not initialize Azure DevOps client: {error}"))
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
}
