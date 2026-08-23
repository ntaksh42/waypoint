//! Azure DevOps 関連のコマンド解析とインデックス項目。

use super::{AZURE_DEVOPS_PREFIX, Action, Entry};

#[derive(Debug, Clone)]
pub(crate) struct AzureIndexed {
    pub(crate) entry: Entry,
    pub(crate) kind: crate::azure_devops::Kind,
    pub(crate) status: String,
    pub(crate) is_mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCommand {
    All,
    PullRequests(PullRequestFilter),
    Pipelines(PipelineFilter),
    Projects,
    WorkItems,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullRequestFilter {
    pub(crate) status: crate::azure_devops::PullRequestStatus,
    pub(crate) mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFilter {
    All,
    Definitions,
    Failed,
}

/// `az` のサブコマンドを分解する。未知の先頭語は検索語として扱うので、
/// `az waypoint` は横断検索、`az pr waypoint` は PR 検索になる。
pub fn azure_command(query: &str) -> Option<(AzureCommand, &str)> {
    let rest = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    let (first, remaining) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(first, remaining)| {
            (first, remaining.trim_start())
        });
    let command = match first.to_ascii_lowercase().as_str() {
        "pr" | "prs" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::All,
            mine: false,
        })),
        "pr-a" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Active,
            mine: false,
        })),
        "pr-c" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Completed,
            mine: false,
        })),
        "pr-ab" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Abandoned,
            mine: false,
        })),
        "pr-mine" | "pr-me" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::All,
            mine: true,
        })),
        "pr-a-mine" | "pr-a-me" => Some(AzureCommand::PullRequests(PullRequestFilter {
            status: crate::azure_devops::PullRequestStatus::Active,
            mine: true,
        })),
        "pipeline" | "pipelines" | "pipe" | "build" | "builds" => {
            Some(AzureCommand::Pipelines(PipelineFilter::All))
        }
        "pipeline-def" | "pipeline-definition" | "pipeline-definitions" => {
            Some(AzureCommand::Pipelines(PipelineFilter::Definitions))
        }
        "pipeline-failed" | "pipeline-fail" | "build-failed" => {
            Some(AzureCommand::Pipelines(PipelineFilter::Failed))
        }
        "project" | "projects" => Some(AzureCommand::Projects),
        "wit" | "wi" | "workitem" | "workitems" => Some(AzureCommand::WorkItems),
        _ => None,
    };
    Some(command.map_or((AzureCommand::All, rest), |command| (command, remaining)))
}

/// 未確定の Azure コマンドだけを補完候補の検索語として取り出す。
/// 例えば `az pln` は `az pipeline` を候補にする一方、`az wp` は通常の
/// Azure 横断検索を維持する。
pub(crate) fn incomplete_azure_command(query: &str) -> Option<&str> {
    let text = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let (command, _) = azure_command(query)?;
    (command == AzureCommand::All).then_some(text)
}

/// `az ` の直後に出すコマンド候補。候補を決定しても検索欄を補完するだけで、
/// URL を開いたり API を呼んだりはしない。
pub(crate) fn azure_command_entries() -> &'static [Entry] {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        [
            ("az pr", "Search all pull requests"),
            ("az pr-a", "Search active pull requests"),
            ("az pr-c", "Search completed pull requests"),
            ("az wit", "Search work items"),
            ("az pipeline", "Search pipelines"),
            ("az project", "Open configured projects"),
        ]
        .into_iter()
        .map(|(name, breadcrumb)| Entry {
            name: name.to_string(),
            breadcrumb: breadcrumb.to_string(),
            path: String::new(),
            action: Action::ReplaceQuery(format!("{name} ")),
            branch: None,
        })
        .collect()
    });
    &ENTRIES
}

pub(crate) fn azure_candidate_entry(candidate: crate::azure_devops::Candidate) -> Entry {
    Entry {
        name: candidate.name,
        breadcrumb: if candidate.aliases.is_empty() {
            candidate.detail
        } else {
            format!("{} — {}", candidate.detail, candidate.aliases.join(" "))
        },
        path: candidate.url.clone(),
        action: Action::OpenUrl(candidate.url),
        branch: None,
    }
}
