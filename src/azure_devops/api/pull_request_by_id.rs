//! PR 番号を指定した Live 検索 — 一覧をスキャンせず PR 単体の API を直接叩く。
//!
//! 一覧 API は作成日の新しい順にページングするため、打ち切り (1 年 / 2000 件)
//! より古い PR には番号が分かっていても辿り着けない。単体取得は古さに関係なく
//! 1 リクエストで当たり、リポジトリ名も応答に含まれるので Web の URL
//! (`.../_git/<repo>/pullrequest/<id>`) をそのまま組み立てられる。

use crate::config::AzureDevOpsProject;

use super::super::cache::CachedRow;
use super::super::convert::{encode_segment, pull_request_row};
use super::http::{API_VERSION, get_json};

/// プロジェクト内で PR を番号で取得する。そのプロジェクトに無ければ `Ok(None)`。
pub(crate) fn fetch_pull_request_by_id(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
    current_user: Option<&str>,
    id: &str,
) -> Result<Option<CachedRow>, String> {
    match get_json(client, &pull_request_by_id_url(project, id), pat) {
        Ok(value) => Ok(pull_request_row(project, &value, current_user)),
        // 監視プロジェクトが複数あると、番号は大抵どれか 1 つにしか無い。
        // 他プロジェクトの 404 を失敗扱いにすると「検索不能」と誤表示する
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn pull_request_by_id_url(project: &AzureDevOpsProject, id: &str) -> String {
    format!(
        "https://dev.azure.com/{}/{}/_apis/git/pullrequests/{}?api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project),
        encode_segment(id.trim()),
    )
}

/// `get_json` は失敗を `HTTP <status>` 入りの文字列で返す。
fn is_not_found(error: &str) -> bool {
    error.contains("HTTP 404")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> AzureDevOpsProject {
        AzureDevOpsProject {
            organization: "aksh0402".to_string(),
            project: "Personal Project".to_string(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        }
    }

    #[test]
    fn url_targets_the_project_level_single_pull_request_endpoint() {
        assert_eq!(
            pull_request_by_id_url(&project(), " 68 "),
            "https://dev.azure.com/aksh0402/Personal%20Project/_apis/git/pullrequests/68?api-version=7.1"
        );
    }

    #[test]
    fn only_404_counts_as_absent() {
        assert!(is_not_found(
            "Azure DevOps request returned HTTP 404 Not Found"
        ));
        assert!(!is_not_found(
            "Azure DevOps request returned HTTP 401 Unauthorized"
        ));
        assert!(!is_not_found("Azure DevOps request failed: timeout"));
    }
}
