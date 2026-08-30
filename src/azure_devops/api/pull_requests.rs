//! Pull Request の取得・共有キャッシュ同期。

use std::time::Duration;

use serde_json::Value;

use crate::config::AzureDevOpsProject;

use super::super::cache::CachedRow;
use super::super::convert::{
    encode_segment, parse_rfc3339_unix, project_url, pull_request_row, repository_names_and_ids,
    unix_timestamp,
};
use super::super::shared_cache::{self, SharedPullRequest, SharedReviewer};
use super::http::{API_VERSION, get_json, http_client};

const PR_PAGE_SIZE: usize = 500;
/// Completed / Abandoned PR はこれより古い作成日のページに達したら打ち切る。
/// Active は状態そのものが「今関心がある」ことの表明なので上限を設けない。
const PR_HISTORY_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 90);
/// 上の日付条件に加えた保険。1 プロジェクトが Completed/Abandoned だけで
/// 何千件もページングし続けないよう件数でも打ち切る。
const PR_HISTORY_MAX_COUNT: usize = 2_000;
/// `az pr` 等がキャッシュ検索で 0 件だったとき、ユーザーが明示的に選んで
/// 叫ぶライブ検索の打ち切り。定期同期の 3 ヶ月より広く 1 年まで遡るが、
/// それでも無制限にはしない (これも実行のたびに数千リクエストしないため)。
const PR_LIVE_SEARCH_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 365);
const PR_LIVE_SEARCH_MAX_COUNT: usize = 2_000;

/// 共有キャッシュ (`shared_cache`) が直近これだけ新しければ、waypoint は
/// Active PR を自分で取得しない。DevDeck 側の同期間隔 (5分) を確実に
/// 拾えるだけの余裕を持たせてある。
pub(super) const SHARED_CACHE_FRESHNESS: Duration = Duration::from_secs(10 * 60);

/// `az` のライブ検索と同じ生 JSON から、共有キャッシュ向けの事実 (PR 本体
/// とレビュアー一覧) を取り出す。`pull_request_row` (waypoint 自身の表示用
/// `name`/`detail` を組み立てる) とは別に、共有キャッシュはどちらのアプリ
/// の表示都合にも寄らない生のフィールドだけを持つ。
fn shared_pull_request_row(
    project: &AzureDevOpsProject,
    item: &Value,
) -> Option<(SharedPullRequest, Vec<SharedReviewer>)> {
    let pull_request_id = item["pullRequestId"].as_i64()?;
    let title = item["title"].as_str()?.to_string();
    let status = item["status"].as_str().unwrap_or("unknown").to_string();
    let repository_id = item["repository"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let repository_name = item["repository"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let created_by = item["createdBy"]["displayName"]
        .as_str()
        .map(str::to_string);
    let created_by_id = item["createdBy"]["id"].as_str().map(str::to_string);
    let creation_date = item["creationDate"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let source_ref_name = short_ref(item["sourceRefName"].as_str().unwrap_or_default());
    let target_ref_name = short_ref(item["targetRefName"].as_str().unwrap_or_default());
    let is_draft = item["isDraft"].as_bool().unwrap_or(false);
    let web_url = item["_links"]["web"]["href"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            Some(format!(
                "{}/_git/{}/pullrequest/{pull_request_id}",
                project_url(project),
                encode_segment(&repository_name)
            ))
        });
    let reviewers = item["reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reviewer| {
            Some(SharedReviewer {
                repository_id: repository_id.clone(),
                pull_request_id,
                reviewer_id: reviewer["id"].as_str()?.to_string(),
                vote: reviewer["vote"].as_i64().unwrap_or(0) as i32,
                is_required: reviewer["isRequired"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    Some((
        SharedPullRequest {
            repository_id,
            repository_name,
            pull_request_id,
            title,
            status,
            created_by,
            created_by_id,
            creation_date,
            source_ref_name,
            target_ref_name,
            is_draft,
            web_url,
        },
        reviewers,
    ))
}

/// `refs/heads/main` のような完全参照名から先頭の `refs/heads/` を落とす。
/// DevDeck 側が書く行と同じ短縮形にして、どちらが書いても表示が揃うようにする。
fn short_ref(ref_name: &str) -> String {
    ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(ref_name)
        .to_string()
}

pub(crate) fn sync_active_prs_to_shared_cache(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<(), String> {
    let organization = project.organization.trim().to_string();
    let project_name = project.project.trim().to_string();
    if let Ok(conn) = shared_cache::open()
        && shared_cache::is_fresh(
            &conn,
            &organization,
            &project_name,
            shared_cache::KIND_PULL_REQUESTS,
            SHARED_CACHE_FRESHNESS,
        )
    {
        return Ok(());
    }

    let mut rows = Vec::new();
    let mut reviewers = Vec::new();
    let mut skip = 0;
    loop {
        let url = pull_requests_url(project, "active", skip, None);
        let value = get_json(client, &url, pat)?;
        let page = value["value"].as_array().cloned().unwrap_or_default();
        let count = page.len();
        for item in &page {
            if let Some((row, item_reviewers)) = shared_pull_request_row(project, item) {
                rows.push(row);
                reviewers.extend(item_reviewers);
            }
        }
        if count < PR_PAGE_SIZE {
            break;
        }
        skip += PR_PAGE_SIZE;
    }

    let mut conn = shared_cache::open()?;
    shared_cache::write_pull_requests(&mut conn, &organization, &project_name, &rows, &reviewers)?;
    shared_cache::mark_synced(
        &conn,
        &organization,
        &project_name,
        shared_cache::KIND_PULL_REQUESTS,
        shared_cache::SYNCED_BY,
    )
}

/// Completed / Abandoned を新しい順に読み、作成日が `PR_HISTORY_MAX_AGE`
/// を超えるか `PR_HISTORY_MAX_COUNT` に達したら打ち切る。プロジェクトの
/// 累積 PR 数が数十万件規模でも、同期が終わらなくなるのを防ぐ (R-13)。
/// Active は DevDeck 側から読むのでここでは取得しない。
///
/// `interest_repositories` が設定されていれば、まずリポジトリ名を GUID へ
/// 解決し `searchCriteria.repositoryId` で絞り込む。名前が現存のリポジトリと
/// 一致しなければそのエントリは無視する (削除・リネームされた場合に同期
/// 全体を失敗させないため)。
pub(crate) fn fetch_pull_request_history(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
) -> Result<Vec<CachedRow>, String> {
    let repository_ids = if project.interest_repositories.is_empty() {
        vec![None]
    } else {
        let repositories = fetch_repositories(client, project, pat)?;
        let ids: Vec<Option<String>> = project
            .interest_repositories
            .iter()
            .filter_map(|name| {
                repositories
                    .iter()
                    .find(|(repo_name, _)| repo_name.eq_ignore_ascii_case(name))
                    .map(|(_, id)| Some(id.clone()))
            })
            .collect();
        if ids.is_empty() { vec![None] } else { ids }
    };

    let mut rows = Vec::new();
    for repository_id in &repository_ids {
        let sync_limits = HistoryLimits {
            max_age: PR_HISTORY_MAX_AGE,
            max_count: PR_HISTORY_MAX_COUNT,
        };
        rows.extend(fetch_pull_requests_history(
            client,
            project,
            pat,
            current_user,
            "completed",
            repository_id.as_deref(),
            sync_limits,
        )?);
        rows.extend(fetch_pull_requests_history(
            client,
            project,
            pat,
            current_user,
            "abandoned",
            repository_id.as_deref(),
            sync_limits,
        )?);
    }
    Ok(rows)
}

/// `az pr` 等がキャッシュ検索で 0 件だったとき、ユーザーが明示的に
/// 選んで叫ぶライブ検索。定期同期の打ち切り (3 ヶ月 / 1000件) を一時的に
/// 大きく緩め、対象ステータスだけ広く取り直してからローカルでキーワード
/// フィルタする。Azure DevOps の PR API に全文検索が無いための代替策。
pub(crate) fn fetch_pull_requests_live(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    status: &str,
) -> Result<Vec<CachedRow>, String> {
    if status.eq_ignore_ascii_case("active") {
        return fetch_pull_requests_by_status(client, project, pat, current_user, status, None);
    }
    fetch_pull_requests_history(
        client,
        project,
        pat,
        current_user,
        status,
        None,
        HistoryLimits {
            max_age: PR_LIVE_SEARCH_MAX_AGE,
            max_count: PR_LIVE_SEARCH_MAX_COUNT,
        },
    )
}

/// プロジェクト内のリポジトリ一覧を (名前, GUID) の組で返す。
pub(crate) fn fetch_repositories(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<(String, String)>, String> {
    let url = format!(
        "https://dev.azure.com/{}/{}/_apis/git/repositories?api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project),
    );
    let value = get_json(client, &url, pat)?;
    Ok(repository_names_and_ids(&value))
}

/// 設定画面用。組織・プロジェクト名と PAT だけからリポジトリ名一覧を取る。
pub fn list_repository_names(
    organization: &str,
    project: &str,
    typed_pat: &str,
) -> Result<Vec<String>, String> {
    let pat = super::super::credential::credential_for_request(organization, typed_pat)?;
    let client = http_client()?;
    let target = crate::config::AzureDevOpsProject {
        organization: organization.to_string(),
        project: project.to_string(),
        aliases: Vec::new(),
        priority: 0,
        include_pull_requests: true,
        include_pipelines: true,
        include_work_items: true,
        interest_areas: Vec::new(),
        interest_repositories: Vec::new(),
    };
    let repositories = fetch_repositories(&client, &target, &pat)?;
    Ok(repositories.into_iter().map(|(name, _)| name).collect())
}

fn pull_requests_url(
    project: &AzureDevOpsProject,
    status: &str,
    skip: usize,
    repository_id: Option<&str>,
) -> String {
    let repository_filter = repository_id
        .map(|id| format!("&searchCriteria.repositoryId={id}"))
        .unwrap_or_default();
    format!(
        "https://dev.azure.com/{}/{}/_apis/git/pullrequests?searchCriteria.status={status}&$top={PR_PAGE_SIZE}&$skip={skip}{repository_filter}&api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project),
    )
}

fn fetch_pull_requests_by_status(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    status: &str,
    repository_id: Option<&str>,
) -> Result<Vec<CachedRow>, String> {
    let mut rows = Vec::new();
    let mut skip = 0;
    loop {
        let url = pull_requests_url(project, status, skip, repository_id);
        let value = get_json(client, &url, pat)?;
        let page = value["value"].as_array().cloned().unwrap_or_default();
        let count = page.len();
        rows.extend(
            page.into_iter()
                .filter_map(|item| pull_request_row(project, &item, current_user)),
        );
        if count < PR_PAGE_SIZE {
            break;
        }
        skip += PR_PAGE_SIZE;
    }
    Ok(rows)
}

/// Completed / Abandoned 用。ページは `creationDate` 降順で返るため、
/// ページ内最古の作成日が上限を超えたらそこで打ち切ってよい。
/// 定期同期用の狭い上限とライブ検索用の広い上限を呼び分けるための組
/// (`fetch_pull_requests` と `fetch_pull_requests_live` 参照)。
#[derive(Clone, Copy)]
struct HistoryLimits {
    max_age: Duration,
    max_count: usize,
}

fn fetch_pull_requests_history(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    status: &str,
    repository_id: Option<&str>,
    limits: HistoryLimits,
) -> Result<Vec<CachedRow>, String> {
    let cutoff = unix_timestamp().saturating_sub(limits.max_age.as_secs() as i64);
    let mut rows = Vec::new();
    let mut skip = 0;
    loop {
        let url = pull_requests_url(project, status, skip, repository_id);
        let value = get_json(client, &url, pat)?;
        let page = value["value"].as_array().cloned().unwrap_or_default();
        let count = page.len();
        let oldest_in_page = page.iter().filter_map(creation_date_unix_from).min();
        rows.extend(
            page.into_iter()
                .filter(|item| {
                    creation_date_unix_from(item).is_none_or(|created| created >= cutoff)
                })
                .filter_map(|item| pull_request_row(project, &item, current_user)),
        );
        if should_stop_history_paging(
            count,
            PR_PAGE_SIZE,
            oldest_in_page,
            cutoff,
            rows.len(),
            limits.max_count,
        ) {
            break;
        }
        skip += PR_PAGE_SIZE;
    }
    rows.truncate(limits.max_count);
    Ok(rows)
}

fn creation_date_unix_from(item: &Value) -> Option<i64> {
    let text = item["creationDate"].as_str()?;
    parse_rfc3339_unix(text)
}

/// Completed / Abandoned のページングを続けるかどうかの純粋な判定。
/// 「ページが埋まりきらなかった (最終ページ)」「ページ内最古の作成日が
/// cutoff を過ぎた」「累積件数が上限に達した」のいずれかで打ち切る。
fn should_stop_history_paging(
    page_len: usize,
    page_size: usize,
    oldest_in_page: Option<i64>,
    cutoff: i64,
    accumulated_len: usize,
    max_count: usize,
) -> bool {
    let reached_cutoff = oldest_in_page.is_some_and(|oldest| oldest < cutoff);
    page_len < page_size || reached_cutoff || accumulated_len >= max_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_paging_continues_while_page_is_full_and_recent() {
        // フルページ (top と同数) かつページ内最古が cutoff より新しければ続行
        assert!(!should_stop_history_paging(
            500,
            500,
            Some(2_000),
            1_000,
            10,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_stops_on_partial_page() {
        // 返ってきた件数が $top 未満 = これが最終ページ
        assert!(should_stop_history_paging(
            120,
            500,
            Some(2_000),
            1_000,
            10,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_stops_once_oldest_item_in_page_predates_cutoff() {
        // ページ内最古の作成日が cutoff (3 ヶ月前など) より古い
        assert!(should_stop_history_paging(
            500,
            500,
            Some(500),
            1_000,
            10,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_stops_at_the_count_cap_even_if_still_recent() {
        assert!(should_stop_history_paging(
            500,
            500,
            Some(2_000),
            1_000,
            PR_HISTORY_MAX_COUNT,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_continues_when_no_page_item_has_a_parsable_date() {
        // creationDate が読めない行しかない場合は cutoff 判定をスキップし、
        // ページが埋まっていれば続行する (安全側 = 取りこぼさない)
        assert!(!should_stop_history_paging(
            500,
            500,
            None,
            1_000,
            10,
            PR_HISTORY_MAX_COUNT
        ));
    }

    #[test]
    fn history_paging_respects_a_custom_max_count_smaller_than_the_default() {
        // ライブ検索用の広い上限だけでなく、狭い上限を渡した場合でも
        // 正しく打ち切れることを確認する
        assert!(should_stop_history_paging(
            500,
            500,
            Some(2_000),
            1_000,
            50,
            50
        ));
        assert!(!should_stop_history_paging(
            500,
            500,
            Some(2_000),
            1_000,
            49,
            50
        ));
    }
}
