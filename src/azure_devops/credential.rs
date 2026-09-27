//! PAT の保存・取得。Windows Credential Manager (`keyring`) を使い、
//! PAT の値は config.json に現れない。

use keyring::Entry as CredentialEntry;

const CREDENTIAL_SERVICE: &str = "Waypoint";

/// Credential Manager 内の組織固有キー。PAT の値は config.json に現れない。
pub(crate) fn credential_key(organization: &str) -> String {
    format!("azure-devops:{}", organization.trim().to_ascii_lowercase())
}

pub fn save_pat(organization: &str, pat: &str) -> Result<(), String> {
    let organization = organization.trim();
    if organization.is_empty() || pat.trim().is_empty() {
        return Err("Organization and PAT are required.".to_string());
    }
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .set_password(pat.trim())
        .map_err(|error| format!("Failed to save PAT: {error}"))
}

pub fn delete_pat(organization: &str) -> Result<(), String> {
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization.trim()))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .delete_credential()
        .or_else(|error| match error {
            keyring::Error::NoEntry => Ok(()),
            error => Err(error),
        })
        .map_err(|error| format!("Failed to delete PAT: {error}"))
}

pub(crate) fn load_pat(organization: &str) -> Result<String, String> {
    CredentialEntry::new(CREDENTIAL_SERVICE, &credential_key(organization))
        .map_err(|error| format!("Credential Manager is unavailable: {error}"))?
        .get_password()
        .map_err(|_| format!("No PAT is saved for Azure DevOps organization \"{organization}\"."))
}

/// この接頭辞で始まる認証情報は PAT ではなく Bearer トークンとして送る。
/// PAT は空白を含まないので取り違えない。
pub(crate) const BEARER_PREFIX: &str = "Bearer ";

/// 保存済みの PAT を優先し、無ければ `az login` 済みの Azure CLI の
/// トークンで代用する (FR-9.18.2)。後者は `az` を起動するので
/// UI スレッドから呼ばないこと。
pub(crate) fn load_credential(organization: &str) -> Result<String, String> {
    load_pat(organization).or_else(|pat_error| {
        super::azure_cli::access_token()
            .map(|token| format!("{BEARER_PREFIX}{token}"))
            .map_err(|cli_error| format!("{pat_error} {cli_error}"))
    })
}

pub(crate) fn credential_for_request(
    organization: &str,
    typed_pat: &str,
) -> Result<String, String> {
    if typed_pat.trim().is_empty() {
        load_credential(organization)
    } else {
        Ok(typed_pat.trim().to_string())
    }
}
