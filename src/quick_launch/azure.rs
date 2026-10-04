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
    WorkItems {
        live: bool,
    },
    /// `az optimize`（`suggest` / `rank` でも入れる）— 直近のアサイン・
    /// メンションから優先 Project / Area を提案する専用画面を開く。
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
    /// `az <query>` (サブコマンド無し) — PR / Work Item / Pipeline を
    /// まとめて Live 検索する。横断検索の候補一覧と対象が揃う。
    AllKinds {
        query: String,
    },
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
    let mut author = false;
    let mut reviewer = false;
    let mut needs_review = false;
    let mut waiting = false;
    let mut draft = false;
    let mut ready = false;
    let mut stale = false;
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
            "author" | "authored" => author = true,
            "reviewer" | "review" => reviewer = true,
            "needs-review" | "needsreview" => needs_review = true,
            "waiting" | "waiting-for-others" => waiting = true,
            "draft" | "drafts" => draft = true,
            "ready" | "ready-to-complete" => ready = true,
            "stale" => stale = true,
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
            author,
            reviewer,
            needs_review,
            waiting,
            draft,
            ready,
            stale,
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
        AzureCommand::All => Some(AzureLiveRequest::AllKinds { query }),
        AzureCommand::Projects | AzureCommand::Suggest => None,
    }
}

/// 検索窓のバッジに出す、サブコマンドと解釈済みの属性トークン。
/// `az pr active mine` → `PR · active · mine`。トークンは入力順ではなく
/// 状態 → 自分との関係の順に並べ、何が解釈されたかを正規化して見せる。
/// サブコマンドを伴わない `az` / `az <検索語>` は `None` (従来どおり
/// `AZURE DEVOPS` のまま)。
pub fn azure_badge_label(query: &str) -> Option<String> {
    let (command, _) = azure_command(query)?;
    let mut parts: Vec<&str> = Vec::new();
    match command {
        AzureCommand::All => return None,
        AzureCommand::PullRequests(filter) => {
            parts.push("PR");
            if filter.status_explicit {
                parts.push(match filter.status {
                    crate::azure_devops::PullRequestStatus::Active => "active",
                    crate::azure_devops::PullRequestStatus::Completed => "completed",
                    crate::azure_devops::PullRequestStatus::Abandoned => "abandoned",
                    crate::azure_devops::PullRequestStatus::All => "all",
                });
            }
            for (on, name) in [
                (filter.mine, "mine"),
                (filter.author, "author"),
                (filter.reviewer, "reviewer"),
                (filter.needs_review, "needs-review"),
                (filter.waiting, "waiting"),
                (filter.draft, "draft"),
                (filter.ready, "ready"),
                (filter.stale, "stale"),
            ] {
                if on {
                    parts.push(name);
                }
            }
        }
        AzureCommand::Pipelines(filter) => {
            parts.push("PIPELINE");
            match filter {
                PipelineFilter::All => {}
                PipelineFilter::Failed => parts.push("failed"),
                PipelineFilter::Definitions => parts.push("definitions"),
            }
        }
        AzureCommand::WorkItems { .. } => parts.push("WORK ITEM"),
        AzureCommand::Projects => parts.push("PROJECT"),
        AzureCommand::Suggest => parts.push("OPTIMIZE"),
    }
    Some(parts.join(" \u{00B7} "))
}

/// `az` の一覧が空になったときに出す説明文 (FR-9.18.7)。
/// 空の一覧は何が起きたのか分からないため、原因と次の一手を示す。
/// `az pr` / `az wit` / `az pipeline` は Live 検索の入口候補を自分で足すので対象外 (`None`)。
pub fn azure_empty_message(query: &str, enabled: bool, has_projects: bool) -> Option<String> {
    let (command, rest) = azure_command(query)?;
    match command {
        AzureCommand::All | AzureCommand::Projects if !enabled => {
            Some("Azure DevOps is disabled. Enable it in Settings.".to_string())
        }
        AzureCommand::All | AzureCommand::Projects if !has_projects => {
            Some("No Azure DevOps projects are configured. Add them in Settings.".to_string())
        }
        AzureCommand::All => Some(format!(
            "No cached matches for \"{}\". Press Ctrl+Enter to search Azure DevOps live.",
            rest.trim()
        )),
        AzureCommand::Projects => Some("No matching projects.".to_string()),
        AzureCommand::PullRequests(_)
        | AzureCommand::Pipelines(_)
        | AzureCommand::WorkItems { .. }
        | AzureCommand::Suggest => None,
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
                    author: false,
                    reviewer: false,
                    needs_review: false,
                    waiting: false,
                    draft: false,
                    ready: false,
                    stale: false,
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
                    author: false,
                    reviewer: false,
                    needs_review: false,
                    waiting: false,
                    draft: false,
                    ready: false,
                    stale: false,
                    live: true,
                }),
                "foo"
            ))
        );
    }

    #[test]
    fn badge_label_shows_the_interpreted_subcommand_and_filters() {
        assert_eq!(azure_badge_label("az "), None);
        assert_eq!(azure_badge_label("az waypoint"), None);
        assert_eq!(azure_badge_label("az pr").as_deref(), Some("PR"));
        // 入力順ではなく状態 → 関係の順に正規化される
        assert_eq!(
            azure_badge_label("az pr mine needs-review active foo").as_deref(),
            Some("PR \u{00B7} active \u{00B7} mine \u{00B7} needs-review")
        );
        // `live` は検索条件ではないのでラベルに出さない
        assert_eq!(azure_badge_label("az pr live").as_deref(), Some("PR"));
        assert_eq!(
            azure_badge_label("az pipeline failed").as_deref(),
            Some("PIPELINE \u{00B7} failed")
        );
        assert_eq!(azure_badge_label("az wit").as_deref(), Some("WORK ITEM"));
        assert_eq!(azure_badge_label("b az"), None);
    }

    #[test]
    fn empty_message_explains_the_cause_and_the_next_step() {
        assert_eq!(
            azure_empty_message("az foo", true, true).as_deref(),
            Some("No cached matches for \"foo\". Press Ctrl+Enter to search Azure DevOps live.")
        );
        assert_eq!(
            azure_empty_message("az foo", false, true).as_deref(),
            Some("Azure DevOps is disabled. Enable it in Settings.")
        );
        assert_eq!(
            azure_empty_message("az foo", true, false).as_deref(),
            Some("No Azure DevOps projects are configured. Add them in Settings.")
        );
        assert_eq!(
            azure_empty_message("az project x", true, true).as_deref(),
            Some("No matching projects.")
        );
        // 自分で Live 検索の入口候補を足すコマンドと、他モードは対象外
        assert_eq!(azure_empty_message("az pr x", true, true), None);
        assert_eq!(azure_empty_message("az wit x", true, true), None);
        assert_eq!(azure_empty_message("b foo", true, true), None);
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
}
