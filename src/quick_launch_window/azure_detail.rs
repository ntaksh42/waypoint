//! `az ` モードの候補の 2 行目 (breadcrumb) を、どのプロジェクト /
//! リポジトリの項目かが一目で分かる形に組み替える。
//!
//! breadcrumb は `Azure DevOps — org/project/repo — active — ...` の形で、
//! 定型の前置きと組織名が幅を取り、肝心のリポジトリ名が途中に埋もれる。
//! 表示だけ `project/repo` を先頭に出して色を変え、残りを中黒で続ける。
//! 検索キー (`LowerKeys`) は元の breadcrumb から作るので影響しない。

use crate::quick_launch::Entry;

const PREFIX: &str = "Azure DevOps — ";
const SEPARATOR: &str = " — ";

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AzureDetail {
    /// `project/repo` (リポジトリを持たない候補は `project`、Project 候補は組織名)。
    pub(super) location: String,
    /// 状態・ブランチ・作成者など。`  ·  ` 区切り。空のこともある。
    pub(super) rest: String,
}

impl AzureDetail {
    /// 1 本の文字列としての表現 (アクセシビリティ用のラベルなど)。
    pub(super) fn joined(&self) -> String {
        if self.rest.is_empty() {
            self.location.clone()
        } else {
            format!("{}{}{}", self.location, DOT, self.rest)
        }
    }
}

pub(super) const DOT: &str = "  \u{00B7}  ";

pub(super) fn azure_detail(entry: &Entry) -> Option<AzureDetail> {
    entry.azure.as_ref()?;
    let body = entry.breadcrumb.strip_prefix(PREFIX)?;
    let mut parts = body.split(SEPARATOR);
    let scope = parts.next()?;
    let segments: Vec<&str> = scope.splitn(3, '/').collect();
    let location = match segments.as_slice() {
        [_, project, repository] => format!("{project}/{repository}"),
        // PR 履歴 (`convert.rs::pull_request_row`) は breadcrumb にリポジトリを
        // 含まないので、URL の `_git/<repo>/` から補う。
        [_, project] => match repository_from_url(&entry.path) {
            Some(repository) => format!("{project}/{repository}"),
            None => (*project).to_string(),
        },
        _ => scope.to_string(),
    };
    Some(AzureDetail {
        location,
        rest: parts.collect::<Vec<_>>().join(DOT),
    })
}

/// `https://dev.azure.com/org/project/_git/<repo>/pullrequest/1` の `<repo>`。
fn repository_from_url(url: &str) -> Option<String> {
    let (_, after) = url.split_once("/_git/")?;
    let segment = after.split(['/', '?', '#']).next()?;
    (!segment.is_empty()).then(|| percent_decode(segment))
}

/// `encode_segment` (`azure_devops::convert`) の逆。不正な `%` はそのまま残す。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure_devops::Kind;
    use crate::quick_launch::{Action, AzureMeta};

    fn entry(breadcrumb: &str, url: &str) -> Entry {
        Entry {
            azure: Some(AzureMeta {
                kind: Kind::PullRequest,
                work_item_type: None,
                work_item_style: None,
                status: "active".into(),
                is_mine: false,
                needs_my_review: false,
                is_draft: false,
            }),
            name: String::new(),
            breadcrumb: breadcrumb.into(),
            path: url.into(),
            action: Action::OpenUrl(url.into()),
            branch: None,
        }
    }

    #[test]
    fn active_pull_requests_lead_with_project_and_repository() {
        let detail = azure_detail(&entry(
            "Azure DevOps — aksh0402/TestProject/TestRepos — active — fix/a → main — by naoto",
            "https://dev.azure.com/aksh0402/TestProject/_git/TestRepos/pullrequest/60",
        ))
        .unwrap();
        assert_eq!(detail.location, "TestProject/TestRepos");
        assert_eq!(
            detail.rest,
            "active  \u{00B7}  fix/a → main  \u{00B7}  by naoto"
        );
    }

    #[test]
    fn history_pull_requests_take_the_repository_from_the_url() {
        let detail = azure_detail(&entry(
            "Azure DevOps — org/Project Name — completed — by Bob",
            "https://dev.azure.com/org/Project%20Name/_git/My%20Repo/pullrequest/7",
        ))
        .unwrap();
        assert_eq!(detail.location, "Project Name/My Repo");
        assert_eq!(detail.rest, "completed  \u{00B7}  by Bob");
    }

    #[test]
    fn work_items_and_projects_keep_what_they_have() {
        let work_item = azure_detail(&entry(
            "Azure DevOps — org/PersonalProject — Task To Do",
            "https://dev.azure.com/org/PersonalProject/_workitems/edit/2",
        ))
        .unwrap();
        assert_eq!(work_item.location, "PersonalProject");
        assert_eq!(work_item.joined(), "PersonalProject  \u{00B7}  Task To Do");

        let project = azure_detail(&entry(
            "Azure DevOps — org",
            "https://dev.azure.com/org/PersonalProject",
        ))
        .unwrap();
        assert_eq!(project.joined(), "org");
    }

    #[test]
    fn non_azure_entries_are_left_alone() {
        let mut plain = entry("Azure DevOps — org/p/r — active", "");
        plain.azure = None;
        assert!(azure_detail(&plain).is_none());
        assert!(azure_detail(&entry("My Special Folders", "")).is_none());
    }

    #[test]
    fn percent_decode_keeps_malformed_sequences() {
        assert_eq!(percent_decode("a%20b%zz%"), "a b%zz%");
    }
}
