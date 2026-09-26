//! PAT が無い組織向けに、`az login` 済みの Azure CLI から Microsoft Entra の
//! アクセストークンを借りる (FR-9.18.2)。`az` の起動は数秒かかるので、
//! バックグラウンドの経路からだけ呼び、期限の手前までトークンを使い回す。

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::System::Threading::CREATE_NO_WINDOW;

/// Azure DevOps のリソース ID。組織を問わず共通。
const AZURE_DEVOPS_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";
/// 期限ぎりぎりのトークンで長い同期を始めないための余裕。
const EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);

struct CachedToken {
    token: String,
    expires_at: SystemTime,
}

/// 保持中はロックしたまま `az` を起動するので、並列の同期スレッドが
/// 同時に何本も `az` を立ち上げることはない。
static TOKEN: Mutex<Option<CachedToken>> = Mutex::new(None);

pub(crate) fn access_token() -> Result<String, String> {
    let mut cached = TOKEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(token) = cached
        .as_ref()
        .filter(|token| SystemTime::now() + EXPIRY_MARGIN < token.expires_at)
    {
        return Ok(token.token.clone());
    }
    let token = fetch_token()?;
    let value = token.token.clone();
    *cached = Some(token);
    Ok(value)
}

fn fetch_token() -> Result<CachedToken, String> {
    // `az` の実体は `az.cmd` で CreateProcessW が直接起動できないため cmd.exe を挟む。
    // CREATE_NO_WINDOW を付けないとコンソールが一瞬開いて閉じる
    let output = Command::new("cmd.exe")
        .args([
            "/c",
            "az",
            "account",
            "get-access-token",
            "--resource",
            AZURE_DEVOPS_RESOURCE,
            "--output",
            "json",
        ])
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|error| format!("Could not run Azure CLI: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().find(|line| !line.trim().is_empty());
        return Err(match reason {
            Some(reason) => format!("Azure CLI sign-in is not available: {}", reason.trim()),
            None => "Azure CLI sign-in is not available. Run `az login`.".to_string(),
        });
    }
    parse_token(&output.stdout)
}

fn parse_token(stdout: &[u8]) -> Result<CachedToken, String> {
    let value: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("Azure CLI returned an invalid token response: {error}"))?;
    let token = value["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or("Azure CLI returned no access token.")?
        .to_string();
    // `expires_on` (Unix 秒) は az 2.54 以降にしかない。無ければ最短の有効期間
    // (1 時間) と見なす
    let expires_at = value["expires_on"]
        .as_u64()
        .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(60 * 60));
    Ok(CachedToken { token, expires_at })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_and_expiry_are_read_from_cli_json() {
        let token =
            parse_token(br#"{"accessToken":"abc","expires_on":1790465797,"tokenType":"Bearer"}"#)
                .unwrap();

        assert_eq!(token.token, "abc");
        assert_eq!(
            token.expires_at,
            UNIX_EPOCH + Duration::from_secs(1_790_465_797)
        );
    }

    #[test]
    fn missing_expiry_falls_back_to_a_short_lifetime() {
        let token = parse_token(br#"{"accessToken":"abc"}"#).unwrap();

        assert!(token.expires_at <= SystemTime::now() + Duration::from_secs(60 * 60));
    }

    #[test]
    fn empty_token_is_rejected() {
        assert!(parse_token(br#"{"accessToken":""}"#).is_err());
        assert!(parse_token(b"not json").is_err());
    }
}
