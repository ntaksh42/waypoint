//! Azure DevOps 関連のコマンド解析とインデックス項目。

use super::search::LowerKeys;
use super::{AZURE_DEVOPS_PREFIX, Action, Entry};

#[derive(Debug, Clone)]
pub(crate) struct AzureIndexed {
    pub(crate) entry: Entry,
    /// `entry` の小文字化済みキャッシュ。`az pr` / `az pipeline` 等の
    /// キー入力のたびに `to_lowercase` を再計算しないための事前計算
    /// (`Index::build` で 1 回だけ作る。folders/apps 等と同じ方針)。
    pub(crate) lower: LowerKeys,
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
///
/// サブコマンドの後ろは属性トークン (`active` / `mine` / `failed` 等) を
/// 空白区切りで好きな順・好きな個数だけ並べられる (`az pr active mine Hoge`)。
/// 未知のトークンに当たった時点でそこから先を検索語とみなす。
pub fn azure_command(query: &str) -> Option<(AzureCommand, &str)> {
    let rest = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    let (first, remaining) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(first, remaining)| {
            (first, remaining.trim_start())
        });
    match first.to_ascii_lowercase().as_str() {
        "pr" | "prs" => Some(parse_pull_request_command(remaining)),
        "pipeline" | "pipelines" | "pipe" | "build" | "builds" => {
            Some(parse_pipeline_command(remaining))
        }
        "project" | "projects" => Some((AzureCommand::Projects, remaining)),
        "wit" | "wi" | "workitem" | "workitems" => Some((AzureCommand::WorkItems, remaining)),
        _ => Some((AzureCommand::All, rest)),
    }
}

/// 空白区切りの先頭トークンを、既知の属性トークンである間だけ剥がしていく。
/// `apply` が `true` を返したトークンだけ消費し、未知のトークンに当たったら
/// そこで止めて残り (検索語) を返す。
fn strip_attribute_tokens(text: &str, mut apply: impl FnMut(&str) -> bool) -> &str {
    let mut rest = text;
    loop {
        let (token, remaining) = rest
            .split_once(char::is_whitespace)
            .map_or((rest, ""), |(token, remaining)| {
                (token, remaining.trim_start())
            });
        if token.is_empty() || !apply(&token.to_ascii_lowercase()) {
            break;
        }
        rest = remaining;
    }
    rest
}

/// `pr` に続く属性トークン (`active` / `completed` / `abandoned` / `mine`)
/// を剥がしていき、未知のトークンからを検索語として返す。
fn parse_pull_request_command(text: &str) -> (AzureCommand, &str) {
    let mut status = crate::azure_devops::PullRequestStatus::All;
    let mut mine = false;
    let rest = strip_attribute_tokens(text, |token| {
        match token {
            "active" => status = crate::azure_devops::PullRequestStatus::Active,
            "completed" | "complete" => status = crate::azure_devops::PullRequestStatus::Completed,
            "abandoned" | "abandon" => status = crate::azure_devops::PullRequestStatus::Abandoned,
            "all" => status = crate::azure_devops::PullRequestStatus::All,
            "mine" | "me" => mine = true,
            _ => return false,
        }
        true
    });
    (
        AzureCommand::PullRequests(PullRequestFilter { status, mine }),
        rest,
    )
}

/// `pipeline` に続く属性トークン (`failed` / `definition`) を剥がしていき、
/// 未知のトークンからを検索語として返す。
fn parse_pipeline_command(text: &str) -> (AzureCommand, &str) {
    let mut filter = PipelineFilter::All;
    let rest = strip_attribute_tokens(text, |token| {
        match token {
            "failed" | "fail" => filter = PipelineFilter::Failed,
            "definition" | "definitions" | "def" => filter = PipelineFilter::Definitions,
            "all" => filter = PipelineFilter::All,
            _ => return false,
        }
        true
    });
    (AzureCommand::Pipelines(filter), rest)
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
            ("az pr", "Search pull requests"),
            ("az wit", "Search work items"),
            ("az pipeline", "Search build pipelines"),
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
