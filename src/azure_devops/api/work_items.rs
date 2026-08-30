//! Work Item の取得・WIQL クエリ・共有キャッシュ同期。

use serde_json::json;

use crate::config::AzureDevOpsProject;

use super::super::Candidate;
use super::super::convert::{
    encode_segment, json_i64, project_url, work_item_batch_candidates, work_item_candidates,
};
use super::super::shared_cache::{self, SharedWorkItem};
use super::http::{API_VERSION, post_json};
use super::pull_requests::SHARED_CACHE_FRESHNESS;

/// `az wit <query>` のライブ全文検索 (ユーザーがキャッシュ検索で見つからず
/// 明示的に追加取得を選んだときだけ呼ぶ) 1 回あたりの上限。
const WORK_ITEM_RESULT_LIMIT: usize = 300;
/// `fetch_work_items` が空クエリで呼ばれた場合に返す「最近更新された
/// Work Item」の上限 (プロジェクトごと)。通常の Quick Launch 表示は
/// 共有キャッシュ (`shared_cache`) を見るのでここへは来ない。
const RECENT_WORK_ITEM_LIMIT: usize = 800;

/// `az` コマンドの優先 Project / Area 提案用。1 プロジェクトあたり、直近
/// `RECENT_ACTIVITY_WINDOW_DAYS` 日以内に自分がアサインされた、または
/// コメント (`System.History`) で @メンションされた Work Item の Area Path
/// を集める (プロジェクトごとの件数は呼び出し側で数える)。
///
/// Azure DevOps Boards の `@recentMentions` マクロは Web UI 専用で WIQL
/// REST API での動作が保証されないため使わず、`System.History CONTAINS @Me`
/// で代替する (`Discussion` は独立フィールドではなく `History` に統合される
/// 仕様: https://learn.microsoft.com/azure/devops/boards/queries/history-and-auditing)。
pub(crate) const RECENT_ACTIVITY_WINDOW_DAYS: i64 = 90;
const RECENT_ACTIVITY_LIMIT: usize = 200;

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

/// `interest_areas` が設定されていれば `UNDER` 条件で WIQL を絞り込む。
/// 空なら従来どおりプロジェクト全体を対象にする。
fn recent_work_items_wiql(project: &AzureDevOpsProject) -> String {
    let base = "SELECT [System.Id] FROM WorkItems ORDER BY [System.ChangedDate] DESC";
    if project.interest_areas.is_empty() {
        return base.to_string();
    }
    let conditions = project
        .interest_areas
        .iter()
        .map(|area| format!("[System.AreaPath] UNDER '{}'", wiql_escape(area)))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!(
        "SELECT [System.Id] FROM WorkItems WHERE {conditions} ORDER BY [System.ChangedDate] DESC"
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

/// `AssignedTo` / `History` はプロジェクト固有のフィールドではないため、
/// URL パスのプロジェクトスコープだけに頼ると組織内の他プロジェクトの
/// Work Item まで返ってくる (実測で確認済み: 3 プロジェクトとも同じ ID
/// 集合が返った)。`[System.TeamProject] = @project` を明示して絞り込む。
fn recent_activity_wiql() -> String {
    format!(
        "SELECT [System.Id] FROM WorkItems WHERE \
         [System.TeamProject] = @project \
         AND [System.ChangedDate] >= @Today - {RECENT_ACTIVITY_WINDOW_DAYS} \
         AND ([System.AssignedTo] = @Me OR [System.History] CONTAINS @Me)"
    )
}

pub(crate) fn fetch_recent_activity_areas(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<String>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let query = post_json(
        client,
        &format!("{base}/_apis/wit/wiql?$top={RECENT_ACTIVITY_LIMIT}&api-version={API_VERSION}"),
        pat,
        &json!({ "query": recent_activity_wiql() }),
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
            "fields": ["System.AreaPath"],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(items["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item["fields"]["System.AreaPath"]
                .as_str()
                .map(str::to_string)
        })
        .collect())
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
    fn recent_activity_wiql_covers_assignment_and_mention_within_the_window() {
        let wiql = recent_activity_wiql();
        assert!(wiql.contains(&format!("@Today - {RECENT_ACTIVITY_WINDOW_DAYS}")));
        assert!(wiql.contains("[System.AssignedTo] = @Me"));
        assert!(wiql.contains("[System.History] CONTAINS @Me"));
    }

    #[test]
    fn recent_activity_wiql_scopes_to_the_current_project() {
        // AssignedTo / History はプロジェクト固有のフィールドではないため、
        // TeamProject を明示しないと組織内の他プロジェクトの Work Item も
        // 返ってくる (実機で確認済みの不具合の再発防止)。
        let wiql = recent_activity_wiql();
        assert!(wiql.contains("[System.TeamProject] = @project"));
    }
}
