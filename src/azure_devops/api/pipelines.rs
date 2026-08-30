//! Pipeline (Build Definition / Build) の取得。永続キャッシュは持たない。

use crate::config::AzureDevOpsProject;

use super::super::cache::CachedRow;
use super::super::convert::{encode_segment, pipeline_build_row, pipeline_definition_row};
use super::http::{API_VERSION, get_json};

const PIPELINE_BUILD_LIMIT: usize = 100;

/// Live 検索 (`az pipeline `) からも使う。Pipeline は永続キャッシュを
/// 持たないので、この結果は呼び出し側でその場だけ表示し保存しない。
pub(crate) fn fetch_pipelines(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<Vec<CachedRow>, String> {
    let base = format!(
        "https://dev.azure.com/{}/{}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let definitions = get_json(
        client,
        &format!("{base}/_apis/build/definitions?$top=1000&api-version={API_VERSION}"),
        pat,
    )?["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let builds = get_json(
        client,
        &format!("{base}/_apis/build/builds?$top={PIPELINE_BUILD_LIMIT}&api-version={API_VERSION}"),
        pat,
    )?["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut rows: Vec<_> = definitions
        .iter()
        .filter_map(|item| pipeline_definition_row(project, item))
        .collect();
    rows.extend(
        builds
            .iter()
            .filter_map(|item| pipeline_build_row(project, item)),
    );
    Ok(rows)
}
