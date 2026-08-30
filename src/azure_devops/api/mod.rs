//! Azure DevOps REST API 呼び出し。

mod areas;
mod http;
mod pipelines;
mod pull_requests;
mod work_items;

use crate::config::AzureDevOpsProject;

use super::cache::{record_project_success, replace_project_cache, write_identity};

pub(crate) use http::{API_VERSION, get_json, http_client};

pub use areas::{fetch_area_nodes, fetch_my_area_suggestions};
pub use pull_requests::list_repository_names;

pub(crate) use pipelines::fetch_pipelines;
pub(crate) use pull_requests::fetch_pull_requests_live;
pub(crate) use work_items::{fetch_recent_activity_areas, fetch_work_items};

/// Active PR と Work Item は共有キャッシュ (`shared_cache`) にだけ書き、
/// waypoint 自身の DB には複製を持たない。共有キャッシュが新しければ
/// (自分でも DevDeck でも直近更新していれば) API を叩かずスキップする。
/// PR の Completed/Abandoned 履歴は共有キャッシュの対象外 (DevDeck は
/// Active しか同期しない) なので、引き続き waypoint 自身の DB に保存する。
pub(crate) fn refresh_project(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<(), String> {
    // 共有キャッシュを読んでスキップする場合でも is_mine の判定に自分の
    // ID が要るので、フェッチの成否とは独立に毎回解決しておく。
    let current_user = current_user_id(client, &project.organization, pat).ok();
    if let Some(user_id) = &current_user {
        let _ = write_identity(project.organization.trim(), user_id);
    }

    let history_rows = if project.include_pull_requests {
        pull_requests::fetch_pull_request_history(client, project, pat, current_user.as_deref())?
    } else {
        Vec::new()
    };
    replace_project_cache(project, &history_rows)?;

    if project.include_pull_requests {
        pull_requests::sync_active_prs_to_shared_cache(client, project, pat)?;
    }
    if project.include_work_items {
        work_items::sync_work_items_to_shared_cache(client, project, pat)?;
    }

    record_project_success(project)
}

pub(crate) fn current_user_id(
    client: &reqwest::blocking::Client,
    organization: &str,
    pat: &str,
) -> Result<String, String> {
    let url = format!(
        "https://dev.azure.com/{}/_apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1&api-version={API_VERSION}",
        super::convert::encode_segment(organization)
    );
    get_json(client, &url, pat)?["authenticatedUser"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Azure DevOps did not return the authenticated user.".to_string())
}
