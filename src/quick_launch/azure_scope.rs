//! PR ショートカットのスコープ。URL エンコードした3セグメントを検索語と分離する。

use crate::azure_devops::encode_url_segment;
use crate::config::AzureDevOpsProject;

pub(crate) fn repository_token(project: &AzureDevOpsProject, repository: &str) -> String {
    format!(
        "repo:{}/{}/{}",
        encode_url_segment(project.organization.trim()),
        encode_url_segment(project.project.trim()),
        encode_url_segment(repository.trim())
    )
}

pub(crate) fn split_repository_query(text: &str) -> (Option<&str>, &str) {
    let text = text.trim_start();
    let (token, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if let Some(scope) = token.strip_prefix("repo:") {
        let parts: Vec<_> = scope.split('/').collect();
        if parts.len() == 3 && parts.iter().all(|part| !part.is_empty()) {
            return (Some(scope), rest.trim_start());
        }
    }
    (None, text)
}

pub(crate) fn repository_matches_project(scope: &str, project: &AzureDevOpsProject) -> bool {
    let mut parts = scope.split('/');
    parts.next().is_some_and(|org| {
        org.eq_ignore_ascii_case(&encode_url_segment(project.organization.trim()))
    }) && parts
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(&encode_url_segment(project.project.trim())))
}

pub(crate) fn repository_matches_url(scope: &str, url: &str) -> bool {
    let Some(path) = url.strip_prefix("https://dev.azure.com/") else {
        return false;
    };
    let mut expected = scope.split('/');
    let mut actual = path.split('/');
    expected
        .next()
        .zip(actual.next())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && expected
            .next()
            .zip(actual.next())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && actual.next() == Some("_git")
        && expected
            .next()
            .zip(actual.next())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && actual.next() == Some("pullrequest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_scope_keeps_spaces_and_reserved_characters_in_one_token() {
        let project = AzureDevOpsProject {
            organization: "org".into(),
            project: "Project Name".into(),
            aliases: Vec::new(),
            priority: 0,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        };
        let token = repository_token(&project, "app/tools");
        let query = format!("{token} fix crash");
        let (scope, term) = split_repository_query(&query);
        assert_eq!(term, "fix crash");
        let scope = scope.unwrap();
        assert!(repository_matches_project(scope, &project));
        assert!(repository_matches_url(
            scope,
            "https://dev.azure.com/org/Project%20Name/_git/app%2Ftools/pullrequest/1"
        ));
        assert!(!repository_matches_url(
            scope,
            "https://dev.azure.com/other/Project%20Name/_git/app%2Ftools/pullrequest/1"
        ));
        assert!(!repository_matches_url(
            scope,
            "https://dev.azure.com/org/Project%20Name/_git/app/pullrequest/1"
        ));
    }
}
