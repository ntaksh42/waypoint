//! Area Path 階層の取得と、自分の担当 Work Item からの Area 提案。

use serde_json::json;

use super::super::convert::{AreaNode, area_nodes, area_path_counts, encode_segment, json_i64};
use super::http::{API_VERSION, get_json, http_client, post_json};

/// 自分に割り当てられた Work Item から件数の多い順に Area Path を推薦する。
/// 設定画面の Area Path ピッカーで「候補を提案」する材料に使う。
const MY_AREA_SUGGESTION_LIMIT: usize = 200;

/// プロジェクトの Area Path 階層をすべて取得する。設定画面のツリーピッカー用。
/// `typed_pat` が空なら Credential Manager に保存済みの PAT を使う。
pub fn fetch_area_nodes(
    organization: &str,
    project: &str,
    typed_pat: &str,
) -> Result<Vec<AreaNode>, String> {
    let pat = super::super::credential::credential_for_request(organization, typed_pat)?;
    let client = http_client()?;
    let url = format!(
        "https://dev.azure.com/{}/{}/_apis/wit/classificationnodes/Areas?$depth=12&api-version={API_VERSION}",
        encode_segment(organization),
        encode_segment(project)
    );
    let value = get_json(&client, &url, &pat)?;
    Ok(area_nodes(&value))
}

pub fn fetch_my_area_suggestions(
    organization: &str,
    project: &str,
    typed_pat: &str,
) -> Result<Vec<(String, usize)>, String> {
    let pat = super::super::credential::credential_for_request(organization, typed_pat)?;
    let client = http_client()?;
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(organization),
        encode_segment(project)
    );
    let query = post_json(
        &client,
        &format!("{base}/_apis/wit/wiql?$top={MY_AREA_SUGGESTION_LIMIT}&api-version={API_VERSION}"),
        &pat,
        &json!({
            "query": "SELECT [System.Id] FROM WorkItems WHERE [System.AssignedTo] = @Me"
        }),
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
        &client,
        &format!("{base}/_apis/wit/workitemsbatch?api-version={API_VERSION}"),
        &pat,
        &json!({
            "ids": ids,
            "fields": ["System.AreaPath"],
            "errorPolicy": "omit"
        }),
    )?;
    Ok(area_path_counts(&items))
}
