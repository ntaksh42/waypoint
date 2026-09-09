//! Work Item の取得・WIQL クエリ・共有キャッシュ同期。

use serde_json::json;

use crate::config::AzureDevOpsProject;

use super::super::Candidate;
use super::super::convert::{
    encode_segment, json_i64, project_url, work_item_batch_candidates, work_item_candidates,
};
use super::super::shared_cache::{self, SharedWorkItem};
use super::http::{API_VERSION, get_json, post_json};
use super::pull_requests::SHARED_CACHE_FRESHNESS;

/// `az wit <query>` のライブ全文検索 (ユーザーがキャッシュ検索で見つからず
/// 明示的に追加取得を選んだときだけ呼ぶ) 1 回あたりの上限。
const WORK_ITEM_RESULT_LIMIT: usize = 300;
/// `fetch_work_items` が空クエリで呼ばれた場合に返す「最近更新された
/// Work Item」の上限 (プロジェクトごと)。通常の Quick Launch 表示は
/// 共有キャッシュ (`shared_cache`) を見るのでここへは来ない。
const RECENT_WORK_ITEM_LIMIT: usize = 800;

/// `az optimize` 用に集計する「最近の作業」の期間。
pub(crate) const RECENT_ACTIVITY_WINDOW_DAYS: i64 = 90;
const WORK_ITEM_BATCH_LIMIT: usize = 200;

/// 直近の自分の作業に含まれる Area / Iteration Path。
pub(crate) struct RecentActivityPaths {
    pub areas: Vec<String>,
    pub iterations: Vec<String>,
}

pub(crate) fn sync_work_items_to_shared_cache(
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
            shared_cache::KIND_WORK_ITEMS,
            SHARED_CACHE_FRESHNESS,
        )
    {
        return Ok(());
    }

    let rows = fetch_work_items_for_shared_cache(client, project, pat)?;
    let mut conn = shared_cache::open()?;
    shared_cache::write_work_items(&mut conn, &organization, &project_name, &rows)?;
    shared_cache::mark_synced(
        &conn,
        &organization,
        &project_name,
        shared_cache::KIND_WORK_ITEMS,
        shared_cache::SYNCED_BY,
    )
}

/// 共有キャッシュ向けの Work Item 取得。`fetch_recent_work_items`
/// (waypoint 自身のライブ検索用、`Candidate` を返す) とは別に、共有先の
/// 他アプリ (DevDeck) が使う `assigned_to` / `tags` などの生フィールドも
/// 合わせて取得する。
fn fetch_work_items_for_shared_cache(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<SharedWorkItem>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let query = post_json(
        client,
        &format!("{base}/_apis/wit/wiql?api-version={API_VERSION}&$top={RECENT_WORK_ITEM_LIMIT}"),
        pat,
        &json!({ "query": recent_work_items_wiql(project) }),
    )?;
    let ids: Vec<i64> = query["workItems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| json_i64(&item["id"]))
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let items = post_json(
        client,
        &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
        pat,
        &json!({
            "ids": ids,
            "fields": [
                "System.Id", "System.Title", "System.State", "System.WorkItemType",
                "System.AssignedTo", "System.ChangedDate", "System.Tags"
            ],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(items["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let fields = &item["fields"];
            let id = json_i64(&fields["System.Id"]).or_else(|| json_i64(&item["id"]))?;
            let title = fields["System.Title"].as_str()?.to_string();
            Some(SharedWorkItem {
                id,
                title,
                work_item_type: fields["System.WorkItemType"].as_str().map(str::to_string),
                state: fields["System.State"].as_str().map(str::to_string),
                assigned_to: fields["System.AssignedTo"]["displayName"]
                    .as_str()
                    .map(str::to_string),
                assigned_to_unique_name: fields["System.AssignedTo"]["uniqueName"]
                    .as_str()
                    .map(str::to_string),
                changed_date: fields["System.ChangedDate"].as_str().map(str::to_string),
                web_url: Some(format!("{}/_workitems/edit/{id}", project_url(project))),
                tags: fields["System.Tags"].as_str().map(str::to_string),
            })
        })
        .collect())
}

pub(crate) fn fetch_work_items(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    query: &str,
) -> Result<Vec<Candidate>, String> {
    if query.trim().is_empty() {
        return fetch_recent_work_items(client, project, pat);
    }
    let url = format!(
        "https://almsearch.dev.azure.com/{}/{}/_apis/search/workitemsearchresults?api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    // ユーザーが明示的に選ぶライブ検索なので、`interest_areas` では絞らず
    // プロジェクト全体を対象にする (`fetch_pull_requests_live` が
    // `interest_repositories` を無視するのと同じ理由)。
    let body = json!({ "searchText": query, "$skip": 0, "$top": WORK_ITEM_RESULT_LIMIT, "includeFacets": false });
    let value = post_json(client, &url, pat, &body)?;
    Ok(work_item_candidates(project, &value))
}

/// Area / Iteration の関心パスが設定されていれば `UNDER` 条件で WIQL を絞り込む。
/// 同じ種類のパスは OR、Area と Iteration の間は AND で結ぶ。どちらも空なら
/// 従来どおりプロジェクト全体を対象にする。
fn recent_work_items_wiql(project: &AzureDevOpsProject) -> String {
    let base = "SELECT [System.Id] FROM WorkItems ORDER BY [System.ChangedDate] DESC";
    let mut clauses = Vec::new();
    if !project.interest_areas.is_empty() {
        let areas = project
            .interest_areas
            .iter()
            .map(|area| format!("[System.AreaPath] UNDER '{}'", wiql_escape(area)))
            .collect::<Vec<_>>()
            .join(" OR ");
        clauses.push(format!("({areas})"));
    }
    if !project.interest_iterations.is_empty() {
        let iterations = project
            .interest_iterations
            .iter()
            .map(|iteration| format!("[System.IterationPath] UNDER '{}'", wiql_escape(iteration)))
            .collect::<Vec<_>>()
            .join(" OR ");
        clauses.push(format!("({iterations})"));
    }
    if clauses.is_empty() {
        return base.to_string();
    }
    format!(
        "SELECT [System.Id] FROM WorkItems WHERE {} ORDER BY [System.ChangedDate] DESC",
        clauses.join(" AND ")
    )
}

/// WIQL の文字列リテラル内でシングルクォートをエスケープする。
fn wiql_escape(value: &str) -> String {
    value.replace('\'', "''")
}

/// WIQL で ID だけを取得し、詳細は batch API で一度に読む。プロジェクトごとの
/// 往復は WIQL の `$top` を指定するかどうかに関わらず二回で収まる。
fn fetch_work_items_by_wiql(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    wiql: &str,
    top: Option<usize>,
) -> Result<Vec<Candidate>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let top_param = top.map_or(String::new(), |top| format!("&$top={top}"));
    let query = post_json(
        client,
        &format!("{base}/_apis/wit/wiql?api-version={API_VERSION}{top_param}"),
        pat,
        &json!({ "query": wiql }),
    )?;
    let ids: Vec<i64> = query["workItems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| json_i64(&item["id"]))
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let items = post_json(
        client,
        &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
        pat,
        &json!({
            "ids": ids,
            "fields": ["System.Id", "System.Title", "System.State", "System.WorkItemType"],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(work_item_batch_candidates(project, &items))
}

/// 空の `az wit` 用に、最近更新された Work Item を WIQL で絞って取得する。
pub(crate) fn fetch_recent_work_items(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<Candidate>, String> {
    fetch_work_items_by_wiql(
        client,
        project,
        pat,
        &recent_work_items_wiql(project),
        Some(RECENT_WORK_ITEM_LIMIT),
    )
}

/// Azure DevOps のアカウント単位「最近の作業」API を使う。WIQL REST API では
/// `@Me` / `@Project` などの UI マクロが展開されないため、これらを含む WIQL
/// では HTTP 400 または空結果になる。応答を対象プロジェクトと直近 90 日で
/// 絞り、必要な Work Item だけを batch API で読み直す。
pub(crate) fn fetch_recent_activity_paths(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<RecentActivityPaths, String> {
    let activity = get_json(
        client,
        &format!(
            "https://dev.azure.com/{}/_apis/work/accountmyworkrecentactivity?api-version={API_VERSION}",
            encode_segment(&project.organization)
        ),
        pat,
    )?;
    let cutoff = recent_activity_cutoff_date();
    let ids = recent_activity_ids(&activity, project.project.trim(), &cutoff);
    if ids.is_empty() {
        return Ok(RecentActivityPaths {
            areas: Vec::new(),
            iterations: Vec::new(),
        });
    }
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let mut paths = RecentActivityPaths {
        areas: Vec::new(),
        iterations: Vec::new(),
    };
    for ids in ids.chunks(WORK_ITEM_BATCH_LIMIT) {
        let items = post_json(
            client,
            &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
            pat,
            &json!({
                "ids": ids,
                "fields": ["System.AreaPath", "System.IterationPath"],
                "errorPolicy": "omit"
            }),
        )?;
        for item in items["value"].as_array().into_iter().flatten() {
            if let Some(path) = item["fields"]["System.AreaPath"].as_str() {
                paths.areas.push(path.to_string());
            }
            if let Some(path) = item["fields"]["System.IterationPath"].as_str() {
                paths.iterations.push(path.to_string());
            }
        }
    }
    Ok(paths)
}

fn recent_activity_ids(activity: &serde_json::Value, project: &str, cutoff: &str) -> Vec<i64> {
    activity
        .as_array()
        .or_else(|| activity["value"].as_array())
        .into_iter()
        .flatten()
        .filter(|item| {
            item["teamProject"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(project))
                && item["activityDate"]
                    .as_str()
                    .and_then(|date| date.get(..10))
                    .is_some_and(|date| date >= cutoff)
        })
        .filter_map(|item| json_i64(&item["id"]))
        .collect()
}

fn recent_activity_cutoff_date() -> String {
    let current_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    date_from_unix_days(current_days.saturating_sub(RECENT_ACTIVITY_WINDOW_DAYS as u64))
}

/// UNIX epoch からの日数を ISO 8601 の日付へ変換する。Azure DevOps の
/// `activityDate` は UTC の ISO 8601 なので、日付部分はこの形式で比較できる。
fn date_from_unix_days(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(interest_areas: Vec<String>) -> AzureDevOpsProject {
        AzureDevOpsProject {
            organization: "org".to_string(),
            project: "project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas,
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        }
    }

    #[test]
    fn recent_work_items_wiql_has_no_where_clause_without_interest_areas() {
        let wiql = recent_work_items_wiql(&project(Vec::new()));
        assert!(!wiql.contains("WHERE"));
    }

    #[test]
    fn recent_work_items_wiql_filters_by_interest_areas() {
        let wiql = recent_work_items_wiql(&project(vec![
            "Waypoint\\Launcher".to_string(),
            "Waypoint\\Search".to_string(),
        ]));
        assert!(wiql.contains("[System.AreaPath] UNDER 'Waypoint\\Launcher'"));
        assert!(wiql.contains("[System.AreaPath] UNDER 'Waypoint\\Search'"));
        assert!(wiql.contains(" OR "));
    }

    #[test]
    fn wiql_escape_doubles_single_quotes() {
        assert_eq!(wiql_escape("O'Brien\\Team"), "O''Brien\\Team");
    }

    #[test]
    fn recent_activity_cutoff_uses_iso_dates() {
        assert_eq!(date_from_unix_days(0), "1970-01-01");
        assert_eq!(date_from_unix_days(20_147), "2025-02-28");
    }

    #[test]
    fn recent_work_items_wiql_filters_by_interest_iterations() {
        let mut project = project(Vec::new());
        project.interest_iterations = vec!["Waypoint\\Sprint 1".to_string()];
        let wiql = recent_work_items_wiql(&project);
        assert!(wiql.contains("[System.IterationPath] UNDER 'Waypoint\\Sprint 1'"));
    }

    #[test]
    fn recent_activity_ids_accepts_the_rest_collection_shape_and_applies_the_window() {
        let activity = json!({ "value": [
            { "id": 1, "teamProject": "Waypoint", "activityDate": "2026-06-11T00:00:00Z" },
            { "id": 2, "teamProject": "Other", "activityDate": "2026-06-11T00:00:00Z" },
            { "id": 3, "teamProject": "Waypoint", "activityDate": "2026-06-09T23:59:59Z" }
        ]});
        assert_eq!(
            recent_activity_ids(&activity, "waypoint", "2026-06-10"),
            [1]
        );
    }
}
