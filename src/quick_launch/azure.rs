//! Azure DevOps 関連のコマンド解析と、索引に載せる候補の型。

use super::search::LowerKeys;
use super::{AZURE_DEVOPS_PREFIX, Entry};

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
    pub(crate) is_author: bool,
    pub(crate) is_reviewer: bool,
    pub(crate) needs_my_review: bool,
    pub(crate) waiting_for_others: bool,
    pub(crate) is_draft: bool,
    pub(crate) ready_to_complete: bool,
    pub(crate) is_stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCommand {
    All,
    PullRequests(PullRequestFilter),
    Pipelines(PipelineFilter),
    Projects,
    New,
    WorkItems(WorkItemFilter),
    /// `az optimize` — 直近の活動から
    /// Project の優先度と Iteration を自動保存する。
    /// 検索対象を持たず確定候補を 1 件だけ返す。
    Suggest,
}

/// `az pr` の状態・自分との関係を表す絞り込み条件。
/// `#[derive(...)]` は Rust の属性構文で、Debug（表示用）、Clone / Copy（複製）、
/// PartialEq / Eq（比較）の標準 trait 実装をコンパイラに自動生成させる。
/// `Default` は絞り込み無し (状態 all・属性なし)。`az <番号>` から PR を
/// 直接引くときに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PullRequestFilter {
    pub(crate) status: crate::azure_devops::PullRequestStatus,
    pub(crate) status_explicit: bool,
    pub(crate) mine: bool,
    pub(crate) author: bool,
    pub(crate) reviewer: bool,
    pub(crate) needs_review: bool,
    pub(crate) waiting: bool,
    pub(crate) draft: bool,
    pub(crate) ready: bool,
    pub(crate) stale: bool,
}

impl PullRequestFilter {
    pub(crate) fn for_live(mut self) -> Self {
        if !self.status_explicit {
            self.status = crate::azure_devops::PullRequestStatus::Active;
        }
        self
    }
}

pub use super::azure_work_item_filter::{WorkItemFilter, WorkItemState};
/// Pipeline は永続キャッシュを持たずライブ検索専用になったため、絞り込み
/// 分類は `azure_devops` 側 (`search_pipelines_live_async` が直接使う) に
/// 定義してここから再エクスポートする。
pub use crate::azure_devops::PipelineFilter;

/// `Ctrl+Enter` で現在の Azure 検索条件を Live 検索へ渡すための要求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzureLiveRequest {
    /// `az <query>` (サブコマンド無し) — PR / Work Item / Pipeline を
    /// まとめて Live 検索する。キャッシュ側の Project は取得対象外。
    AllKinds { query: String },
    WorkItems {
        filter: WorkItemFilter,
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
    // `az "build fix"`: `"` で始めたらサブコマンド判定をせず横断検索の検索語にする。
    // コマンド名と同じ語 (build / new / project 等) を検索する逃げ道
    if let Some(quoted) = rest.strip_prefix('"') {
        return Some((
            AzureCommand::All,
            quoted.strip_suffix('"').unwrap_or(quoted),
        ));
    }
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
        "new" => Some((AzureCommand::New, remaining)),
        "project" | "projects" => Some((AzureCommand::Projects, remaining)),
        "wit" | "wi" | "workitem" | "workitems" => Some(parse_work_item_command(remaining)),
        "optimize" => Some((AzureCommand::Suggest, remaining)),
        _ => Some((AzureCommand::All, rest)),
    }
}

/// 空白区切りの先頭トークンを、既知の属性トークンである間だけ剥がしていく。
/// `apply` が `true` を返したトークンだけ消費し、未知のトークンに当たったら
/// そこで止めて残り (検索語) を返す。
pub(super) fn strip_attribute_tokens(text: &str, mut apply: impl FnMut(&str) -> bool) -> &str {
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

/// `az pr` の属性トークン 1 個を `filter` へ反映する。属性でなければ `false`。
/// `token` は小文字化済み。`repo:` の後ろの属性を読み飛ばす
/// (`split_repository_query`) ときにも同じ判定を使う。
pub(super) fn apply_pull_request_attribute(filter: &mut PullRequestFilter, token: &str) -> bool {
    use crate::azure_devops::PullRequestStatus;
    let status = match token {
        "active" => Some(PullRequestStatus::Active),
        "completed" | "complete" => Some(PullRequestStatus::Completed),
        "abandoned" | "abandon" => Some(PullRequestStatus::Abandoned),
        "all" => Some(PullRequestStatus::All),
        _ => None,
    };
    if let Some(status) = status {
        filter.status = status;
        filter.status_explicit = true;
        return true;
    }
    match token {
        "mine" | "me" => filter.mine = true,
        "author" | "authored" => filter.author = true,
        "reviewer" | "review" => filter.reviewer = true,
        "needs-review" | "needsreview" => filter.needs_review = true,
        "waiting" | "waiting-for-others" => filter.waiting = true,
        "draft" | "drafts" => filter.draft = true,
        "ready" | "ready-to-complete" => filter.ready = true,
        "stale" => filter.stale = true,
        _ => return false,
    }
    true
}

/// `pr` に続く属性トークンを剥がしていき、未知のトークンからを検索語として返す。
///
/// `repo:<org>/<project>/<repo>` も属性と同じく順不同で置ける。ただし範囲は
/// 返す文字列の先頭に残す必要がある (呼び出し側が `split_repository_query` で
/// 取り出す) ので、`repo:` があれば戻り値はそこから始め、その後ろに並ぶ属性は
/// `split_repository_query` が読み飛ばす。
fn parse_pull_request_command(text: &str) -> (AzureCommand, &str) {
    let mut filter = PullRequestFilter::default();
    let mut rest = text;
    let mut repository: Option<&str> = None;
    loop {
        let (token, remaining) = rest
            .split_once(char::is_whitespace)
            .map_or((rest, ""), |(token, remaining)| {
                (token, remaining.trim_start())
            });
        if token.is_empty() {
            break;
        }
        if repository.is_none() && super::azure_scope::repository_scope(token).is_some() {
            repository = Some(rest);
        } else if !apply_pull_request_attribute(&mut filter, &token.to_ascii_lowercase()) {
            break;
        }
        rest = remaining;
    }
    (
        AzureCommand::PullRequests(filter),
        repository.unwrap_or(rest),
    )
}

/// `wit` に続く属性トークン (`mine` / `bug` / `open` 等) を剥がしていき、
/// 未知のトークンからを検索語として返す。
fn parse_work_item_command(text: &str) -> (AzureCommand, &str) {
    let mut filter = WorkItemFilter::default();
    let rest = strip_attribute_tokens(text, |token| filter.apply(token));
    (AzureCommand::WorkItems(filter), rest)
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
        AzureCommand::WorkItems(filter) => Some(AzureLiveRequest::WorkItems { filter, query }),
        AzureCommand::PullRequests(filter) => {
            let filter = filter.for_live();
            Some(AzureLiveRequest::PullRequests { filter, query })
        }
        AzureCommand::Pipelines(filter) => Some(AzureLiveRequest::Pipelines { filter, query }),
        AzureCommand::All => Some(AzureLiveRequest::AllKinds { query }),
        AzureCommand::Projects | AzureCommand::Suggest | AzureCommand::New => None,
    }
}

/// 未確定の Azure コマンドだけを補完候補の検索語として取り出す。
/// 例えば `az pln` は `az pipeline` を候補にする一方、`az wp` は通常の
/// Azure 横断検索を維持する。
pub(crate) fn incomplete_azure_command(query: &str) -> Option<&str> {
    let text = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    if text.is_empty() || text.starts_with('"') || text.contains(char::is_whitespace) {
        return None;
    }
    let (command, _) = azure_command(query)?;
    (command == AzureCommand::All).then_some(text)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_live_search_accepts_empty_work_item_query() {
        assert_eq!(
            azure_live_request("az wit"),
            Some(AzureLiveRequest::WorkItems {
                filter: WorkItemFilter::default(),
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
                    author: false,
                    reviewer: false,
                    needs_review: false,
                    waiting: false,
                    draft: false,
                    ready: false,
                    stale: false,
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
    fn live_token_is_no_longer_an_attribute() {
        assert_eq!(
            azure_command("az wit live foo"),
            Some((
                AzureCommand::WorkItems(WorkItemFilter::default()),
                "live foo"
            ))
        );
        let Some((AzureCommand::PullRequests(filter), rest)) = azure_command("az pr mine live foo")
        else {
            panic!("expected pull request command");
        };
        assert!(filter.mine);
        assert_eq!(rest, "live foo");
    }

    #[test]
    fn work_item_attributes_are_stripped_in_any_order() {
        let Some((AzureCommand::WorkItems(filter), rest)) =
            azure_command("az wit bug mine open crash dump")
        else {
            panic!("expected work item command");
        };
        assert!(filter.mine);
        assert_eq!(filter.state, WorkItemState::Open);
        assert!(filter.is_filtered());
        assert_eq!(rest, "crash dump");
        let Some(AzureLiveRequest::WorkItems {
            filter: live,
            query,
        }) = azure_live_request("az wit bug mine open crash dump")
        else {
            panic!("expected work item live search");
        };
        assert_eq!(live, filter);
        assert_eq!(query, "crash dump");
    }

    #[test]
    fn optimize_subcommand_has_no_search_term_and_no_aliases() {
        assert_eq!(
            azure_command("az optimize"),
            Some((AzureCommand::Suggest, ""))
        );
        // 別名 `suggest` / `rank` は一般語なので廃止した。横断検索の検索語になる
        assert_eq!(
            azure_command("az suggest"),
            Some((AzureCommand::All, "suggest"))
        );
        assert_eq!(azure_command("az rank"), Some((AzureCommand::All, "rank")));
        // 属性トークンや検索語を持たないコマンドなので、余分な文字列が
        // 付いても後続はそのまま無視されずに残る (呼び出し側が捨てる)。
        assert_eq!(
            azure_command("az optimize ignored"),
            Some((AzureCommand::Suggest, "ignored"))
        );
    }
    #[test]
    fn repository_scope_can_be_placed_before_or_after_attributes() {
        for query in [
            "az pr repo:o/p/r mine active foo bar",
            "az pr mine repo:o/p/r active foo bar",
            "az pr mine active repo:o/p/r foo bar",
        ] {
            let Some((AzureCommand::PullRequests(filter), rest)) = azure_command(query) else {
                panic!("expected pull request command: {query}");
            };
            assert!(filter.mine, "{query}");
            assert_eq!(
                filter.status,
                crate::azure_devops::PullRequestStatus::Active,
                "{query}"
            );
            assert_eq!(
                crate::quick_launch::split_repository_query(rest),
                (Some("o/p/r"), "foo bar"),
                "{query}"
            );
        }
    }

    #[test]
    fn second_repository_token_and_words_after_the_term_stay_in_the_term() {
        let Some((AzureCommand::PullRequests(filter), rest)) =
            azure_command("az pr repo:o/p/r fix mine")
        else {
            panic!("expected pull request command");
        };
        assert!(!filter.mine);
        assert_eq!(
            crate::quick_launch::split_repository_query(rest),
            (Some("o/p/r"), "fix mine")
        );
    }

    #[test]
    fn leading_quote_searches_command_names_as_plain_words() {
        assert_eq!(
            azure_command("az \"build fix\""),
            Some((AzureCommand::All, "build fix"))
        );
        assert_eq!(
            azure_command("az \"new feature"),
            Some((AzureCommand::All, "new feature"))
        );
        assert_eq!(
            azure_live_request("az \"build\""),
            Some(AzureLiveRequest::AllKinds {
                query: "build".to_string(),
            })
        );
        assert_eq!(incomplete_azure_command("az \"pip"), None);
    }
}
