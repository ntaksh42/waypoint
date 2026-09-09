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
    WorkItems {
        live: bool,
    },
    /// `az optimize`（`suggest` / `rank` でも入れる）— 直近のアサイン・
    /// メンションから優先 Project / Area を提案する専用画面を開く。
    /// 検索対象を持たず確定候補を 1 件だけ返す。
    Suggest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullRequestFilter {
    pub(crate) status: crate::azure_devops::PullRequestStatus,
    pub(crate) status_explicit: bool,
    pub(crate) mine: bool,
    /// `live` トークンの解析互換性を保つ。API 呼び出しは入力変更ではなく
    /// `Ctrl+Enter` の明示操作でのみ開始する。
    pub(crate) live: bool,
}

impl PullRequestFilter {
    pub(crate) fn for_live(mut self) -> Self {
        if !self.status_explicit {
            self.status = crate::azure_devops::PullRequestStatus::Active;
        }
        self
    }
}

/// Pipeline は永続キャッシュを持たずライブ検索専用になったため、絞り込み
/// 分類は `azure_devops` 側 (`search_pipelines_live_async` が直接使う) に
/// 定義してここから再エクスポートする。
pub use crate::azure_devops::PipelineFilter;

/// `Ctrl+Enter` で現在の Azure 検索条件を Live 検索へ渡すための要求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzureLiveRequest {
    WorkItems {
        query: String,
    },
    PullRequests {
        filter: PullRequestFilter,
        query: String,
    },
    Pipelines {
        filter: PipelineFilter,
        query: String,
    },
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
        "wit" | "wi" | "workitem" | "workitems" => Some(parse_work_item_command(remaining)),
        "optimize" | "suggest" | "rank" => Some((AzureCommand::Suggest, remaining)),
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

/// `pr` に続く属性トークン (`active` / `completed` / `abandoned` / `mine` /
/// `live`) を剥がしていき、未知のトークンからを検索語として返す。
fn parse_pull_request_command(text: &str) -> (AzureCommand, &str) {
    let mut status = crate::azure_devops::PullRequestStatus::All;
    let mut status_explicit = false;
    let mut mine = false;
    let mut live = false;
    let rest = strip_attribute_tokens(text, |token| {
        match token {
            "active" => {
                status = crate::azure_devops::PullRequestStatus::Active;
                status_explicit = true;
            }
            "completed" | "complete" => {
                status = crate::azure_devops::PullRequestStatus::Completed;
                status_explicit = true;
            }
            "abandoned" | "abandon" => {
                status = crate::azure_devops::PullRequestStatus::Abandoned;
                status_explicit = true;
            }
            "all" => {
                status = crate::azure_devops::PullRequestStatus::All;
                status_explicit = true;
            }
            "mine" | "me" => mine = true,
            "live" => live = true,
            _ => return false,
        }
        true
    });
    (
        AzureCommand::PullRequests(PullRequestFilter {
            status,
            status_explicit,
            mine,
            live,
        }),
        rest,
    )
}

/// `wit` に続く属性トークン (`live`) を剥がしていき、未知のトークンから
/// を検索語として返す。
fn parse_work_item_command(text: &str) -> (AzureCommand, &str) {
    let mut live = false;
    let rest = strip_attribute_tokens(text, |token| {
        match token {
            "live" => live = true,
            _ => return false,
        }
        true
    });
    (AzureCommand::WorkItems { live }, rest)
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

/// 現在の入力を、ユーザーが明示的に実行する Live 検索要求へ変換する。
/// 検索語が空でも「最近の項目を今取得する」操作として許可する。
pub fn azure_live_request(query: &str) -> Option<AzureLiveRequest> {
    let (command, rest) = azure_command(query)?;
    let query = rest.trim().to_string();
    match command {
        AzureCommand::WorkItems { .. } => Some(AzureLiveRequest::WorkItems { query }),
        AzureCommand::PullRequests(filter) => {
            let filter = filter.for_live();
            Some(AzureLiveRequest::PullRequests { filter, query })
        }
        AzureCommand::Pipelines(filter) => Some(AzureLiveRequest::Pipelines { filter, query }),
        AzureCommand::All | AzureCommand::Projects | AzureCommand::Suggest => None,
    }
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

/// `az ` の直後に出すコマンド候補。`pr` / `wit` / `pipeline` / `project` は
/// 検索対象を持つサブコマンドなので選んでも検索欄を補完するだけだが、
/// `optimize` は検索を挟まない単一アクションなので、選んだ時点で
/// `AzureOptimize` を直接実行する (`az optimize` とフルタイプして
/// Enter した場合と同じ 1 手で済ませる — 補完してからもう一度 Enter する
/// 二度手間を避ける)。
pub(crate) fn azure_command_entries() -> &'static [Entry] {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        let mut entries: Vec<Entry> = [
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
        .collect();
        entries.push(azure_suggest_entry());
        entries
    });
    &ENTRIES
}

/// `az optimize`（`suggest` / `rank` でも入れる）に入ったときの唯一の確定候補。
/// 検索対象を持たないコマンドなので、選択すると直近アクティビティから
/// Project / Iteration の優先度をバックグラウンドで自動更新する。
pub(crate) fn azure_suggest_entry() -> Entry {
    Entry {
        name: "az optimize".to_string(),
        breadcrumb: "Automatically rank projects & iterations from recent activity".to_string(),
        path: String::new(),
        action: Action::AzureOptimize,
        branch: None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_live_search_accepts_empty_work_item_query() {
        assert_eq!(
            azure_live_request("az wit"),
            Some(AzureLiveRequest::WorkItems {
                query: String::new(),
            })
        );
    }

    #[test]
    fn manual_live_search_preserves_pull_request_filters() {
        assert_eq!(
            azure_live_request("az pr active mine waypoint"),
            Some(AzureLiveRequest::PullRequests {
                filter: PullRequestFilter {
                    status: crate::azure_devops::PullRequestStatus::Active,
                    status_explicit: true,
                    mine: true,
                    live: false,
                },
                query: "waypoint".to_string(),
            })
        );
    }

    #[test]
    fn manual_live_search_defaults_pull_requests_to_active_but_honors_explicit_all() {
        let Some(AzureLiveRequest::PullRequests { filter, .. }) =
            azure_live_request("az pr waypoint")
        else {
            panic!("expected pull request live search");
        };
        assert_eq!(
            filter.status,
            crate::azure_devops::PullRequestStatus::Active
        );

        let Some(AzureLiveRequest::PullRequests { filter, .. }) =
            azure_live_request("az pr all waypoint")
        else {
            panic!("expected pull request live search");
        };
        assert_eq!(filter.status, crate::azure_devops::PullRequestStatus::All);
    }

    #[test]
    fn manual_live_search_supports_pipeline_and_rejects_other_modes() {
        assert_eq!(
            azure_live_request("az pipeline failed"),
            Some(AzureLiveRequest::Pipelines {
                filter: PipelineFilter::Failed,
                query: String::new(),
            })
        );
        assert_eq!(azure_live_request("az project waypoint"), None);
        assert_eq!(azure_live_request("waypoint"), None);
    }

    #[test]
    fn live_token_is_parsed_without_becoming_part_of_the_search_term() {
        assert_eq!(
            azure_command("az wit live foo"),
            Some((AzureCommand::WorkItems { live: true }, "foo"))
        );
        assert_eq!(
            azure_command("az pr live mine foo"),
            Some((
                AzureCommand::PullRequests(PullRequestFilter {
                    status: crate::azure_devops::PullRequestStatus::All,
                    status_explicit: false,
                    mine: true,
                    live: true,
                }),
                "foo"
            ))
        );
    }

    #[test]
    fn optimize_subcommand_and_its_aliases_have_no_search_term() {
        assert_eq!(
            azure_command("az optimize"),
            Some((AzureCommand::Suggest, ""))
        );
        assert_eq!(
            azure_command("az suggest"),
            Some((AzureCommand::Suggest, ""))
        );
        assert_eq!(azure_command("az rank"), Some((AzureCommand::Suggest, "")));
        // 属性トークンや検索語を持たないコマンドなので、余分な文字列が
        // 付いても後続はそのまま無視されずに残る (呼び出し側が捨てる)。
        assert_eq!(
            azure_command("az optimize ignored"),
            Some((AzureCommand::Suggest, "ignored"))
        );
    }

    #[test]
    fn azure_suggest_entry_triggers_the_suggest_priorities_action() {
        let entry = azure_suggest_entry();
        assert_eq!(entry.action, Action::AzureOptimize);
        assert!(entry.path.is_empty());
    }

    #[test]
    fn az_optimize_is_offered_among_the_top_level_command_completions() {
        assert!(
            azure_command_entries()
                .iter()
                .any(|entry| entry.name == "az optimize")
        );
    }
}
