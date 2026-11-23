//! `az pr` / `az wit` / `az pipeline` の属性一覧 (FR-9.18 属性表)。
//!
//! 絞り込みそのものは `azure.rs` / `azure_work_item_filter.rs` が行う。ここでは
//! 入力から「どの属性として読んだか」を取り出し、説明行の札と補完候補を作る。
//! 別名の表は解析側と二重に持つので、`tests` で解析側が全部受け付けることを確かめる。

use super::{AZURE_DEVOPS_PREFIX, Action, AzureCommand, Entry, azure_command};

/// 属性 1 個。`name` は札と補完に出す正式名、`aliases` は解析が受け付ける別名。
pub(crate) struct Attribute {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    description: &'static str,
}

const fn attr(
    name: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
) -> Attribute {
    Attribute {
        name,
        aliases,
        description,
    }
}

pub(crate) static PULL_REQUEST: &[Attribute] = &[
    attr("active", &[], "Active PRs"),
    attr("completed", &["complete"], "Completed PRs"),
    attr("abandoned", &["abandon"], "Abandoned PRs"),
    attr("all", &[], "PRs in every state"),
    attr("mine", &["me"], "PRs I authored or review"),
    attr("author", &["authored"], "PRs I authored"),
    attr("reviewer", &["review"], "PRs I review"),
    attr("needs-review", &["needsreview"], "PRs awaiting my vote"),
    attr(
        "waiting",
        &["waiting-for-others"],
        "My PRs awaiting other reviewers",
    ),
    attr("draft", &["drafts"], "Draft PRs"),
    attr("ready", &["ready-to-complete"], "PRs ready to complete"),
    attr("stale", &[], "PRs open for 14 days or more"),
];

pub(crate) static WORK_ITEM: &[Attribute] = &[
    attr("mine", &["me"], "Work items assigned to me"),
    attr("bug", &["bugs"], "Bugs"),
    attr("task", &["tasks"], "Tasks"),
    attr(
        "story",
        &["stories"],
        "User Stories / Product Backlog Items / Requirements",
    ),
    attr("feature", &["features"], "Features"),
    attr("epic", &["epics"], "Epics"),
    attr("open", &[], "Work items not yet closed"),
    attr("closed", &[], "Closed / Done / Removed work items"),
];

pub(crate) static PIPELINE: &[Attribute] = &[
    attr("failed", &["fail"], "Failed builds"),
    attr(
        "definition",
        &["definitions", "def"],
        "Pipeline definitions",
    ),
    attr("all", &[], "Definitions and recent builds"),
];

/// 入力を属性の並びとして読んだ結果。
struct Parsed<'a> {
    attributes: &'static [Attribute],
    /// 読んだ属性の札 (正式名か `repo:` トークン)。重複は除く。
    labels: Vec<String>,
    /// 属性の後ろ (検索語)。`query` の末尾を指す部分文字列。
    term: &'a str,
}

fn parse(query: &str) -> Option<Parsed<'_>> {
    let command = azure_command(query)?.0;
    let attributes = match command {
        AzureCommand::PullRequests(_) => PULL_REQUEST,
        AzureCommand::WorkItems(_) => WORK_ITEM,
        AzureCommand::Pipelines(_) => PIPELINE,
        _ => return None,
    };
    let is_pull_request = matches!(command, AzureCommand::PullRequests(_));
    let is_pipeline = matches!(command, AzureCommand::Pipelines(_));
    let after_prefix = query.strip_prefix(AZURE_DEVOPS_PREFIX)?;
    // サブコマンド名を飛ばす。`azure_command` が上で一致を確かめている
    let mut rest = after_prefix
        .split_once(char::is_whitespace)
        .map_or("", |(_, remaining)| remaining.trim_start());
    let mut labels: Vec<String> = Vec::new();
    let mut has_repository = false;
    loop {
        let (token, remaining) = rest
            .split_once(char::is_whitespace)
            .map_or((rest, ""), |(token, remaining)| {
                (token, remaining.trim_start())
            });
        if token.is_empty() || token.starts_with('"') {
            break;
        }
        let lower = token.to_ascii_lowercase();
        let label = if is_pull_request
            && !has_repository
            && super::azure_scope::repository_scope(token).is_some()
        {
            has_repository = true;
            token.to_string()
        } else if let Some(found) = lookup(attributes, &lower) {
            found.name.to_string()
        } else {
            break;
        };
        // Pipeline の属性は排他で、最後に書いたものが効く (`parse_pipeline_command`)
        if is_pipeline {
            labels.clear();
        }
        if !labels.contains(&label) {
            labels.push(label);
        }
        rest = remaining;
    }
    Some(Parsed {
        attributes,
        labels,
        term: rest,
    })
}

fn lookup<'a>(attributes: &'a [Attribute], token: &str) -> Option<&'a Attribute> {
    attributes
        .iter()
        .find(|attribute| attribute.name == token || attribute.aliases.contains(&token))
}

/// 結果の先頭の説明行に出す属性の文言。属性を読んだら札を並べ、属性も検索語も
/// 無ければ使える属性の一覧を出す。どちらでもなければ `None`。
pub(crate) fn azure_attribute_notice(query: &str) -> Option<String> {
    let parsed = parse(query)?;
    if !parsed.labels.is_empty() {
        let chips: Vec<String> = parsed
            .labels
            .iter()
            .map(|label| format!("[{label}]"))
            .collect();
        return Some(format!("Filters: {}", chips.join(" ")));
    }
    if parsed.term.trim().is_empty() {
        let names: Vec<&str> = parsed.attributes.iter().map(|a| a.name).collect();
        return Some(format!("Filters: {}", names.join(" ")));
    }
    None
}

/// 属性を置ける位置で途中まで打った語に前方一致する属性を、補完候補にする。
/// 選ぶと入力欄をその属性まで補う (`Action::ReplaceQuery`)。
pub(crate) fn azure_attribute_completions(query: &str) -> Vec<Entry> {
    let Some(parsed) = parse(query) else {
        return Vec::new();
    };
    let partial = parsed.term;
    if partial.is_empty()
        || partial.contains(char::is_whitespace)
        || partial.starts_with('"')
        || partial.contains(':')
    {
        return Vec::new();
    }
    let lower = partial.to_ascii_lowercase();
    let head = &query[..query.len() - partial.len()];
    parsed
        .attributes
        .iter()
        .filter(|attribute| {
            attribute.name.starts_with(&lower)
                && attribute.name != lower
                && !parsed.labels.iter().any(|label| label == attribute.name)
        })
        .map(|attribute| Entry {
            azure: None,
            name: format!("{head}{}", attribute.name),
            breadcrumb: format!("Filter: {}", attribute.description),
            path: String::new(),
            action: Action::ReplaceQuery(format!("{head}{} ", attribute.name)),
            branch: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表の正式名と別名を、解析側 (`azure_command`) がすべて属性として読むこと。
    #[test]
    fn every_listed_attribute_is_accepted_by_the_parser() {
        for (command, attributes) in [
            ("pr", PULL_REQUEST),
            ("wit", WORK_ITEM),
            ("pipeline", PIPELINE),
        ] {
            for attribute in attributes {
                for token in std::iter::once(&attribute.name).chain(attribute.aliases) {
                    let query = format!("az {command} {token} foo");
                    let (_, rest) = azure_command(&query).unwrap();
                    assert_eq!(rest, "foo", "{query}");
                }
            }
        }
    }

    #[test]
    fn notice_lists_parsed_attributes_with_canonical_names() {
        assert_eq!(
            azure_attribute_notice("az pr drafts me fix").as_deref(),
            Some("Filters: [draft] [mine]")
        );
        assert_eq!(
            azure_attribute_notice("az pr mine repo:o/p/r fix").as_deref(),
            Some("Filters: [mine] [repo:o/p/r]")
        );
        assert_eq!(
            azure_attribute_notice("az wit bug bugs open").as_deref(),
            Some("Filters: [bug] [open]")
        );
        assert_eq!(
            azure_attribute_notice("az pipeline failed def").as_deref(),
            Some("Filters: [definition]")
        );
    }

    #[test]
    fn notice_lists_available_attributes_when_nothing_is_typed() {
        assert_eq!(
            azure_attribute_notice("az pipeline ").as_deref(),
            Some("Filters: failed definition all")
        );
        assert_eq!(azure_attribute_notice("az pr fix"), None);
        assert_eq!(azure_attribute_notice("az pr \"draft\""), None);
        assert_eq!(azure_attribute_notice("az fix"), None);
        assert_eq!(azure_attribute_notice("az project "), None);
    }

    #[test]
    fn completions_follow_the_partial_attribute() {
        let names = |query: &str| -> Vec<String> {
            azure_attribute_completions(query)
                .into_iter()
                .map(|entry| entry.name)
                .collect()
        };
        assert_eq!(names("az pr dr"), ["az pr draft"]);
        assert_eq!(
            names("az pr mine a"),
            [
                "az pr mine active",
                "az pr mine abandoned",
                "az pr mine all",
                "az pr mine author"
            ]
        );
        // 指定済み・完全一致・語の確定後・引用・`repo:` は出さない
        assert!(
            names("az pr active a")
                .iter()
                .all(|n| !n.ends_with("active"))
        );
        assert!(names("az pr draft").is_empty());
        assert!(names("az pr fix ").is_empty());
        assert!(names("az pr fix d").is_empty());
        assert!(names("az pr \"d").is_empty());
        assert!(names("az pr repo:").is_empty());
        assert!(names("az d").is_empty());
        let entry = &azure_attribute_completions("az wit B")[0];
        assert_eq!(entry.name, "az wit bug");
        assert_eq!(entry.action, Action::ReplaceQuery("az wit bug ".into()));
    }
}

#[cfg(test)]
mod quote_tests {
    use super::super::{AzureCommand, azure_command, split_repository_query};

    /// 検索語全体を `"` で囲むと、属性名と同じ語も検索語になる。
    #[test]
    fn quoted_term_stops_attribute_parsing() {
        let Some((AzureCommand::PullRequests(filter), rest)) =
            azure_command("az pr mine \"draft fix\"")
        else {
            panic!("expected pr command");
        };
        assert!(filter.mine && !filter.draft);
        assert_eq!(rest, "draft fix");

        let Some((AzureCommand::WorkItems(filter), rest)) = azure_command("az wit \"bug\"") else {
            panic!("expected wit command");
        };
        assert!(!filter.is_filtered());
        assert_eq!(rest, "bug");

        let Some((AzureCommand::Pipelines(_), rest)) = azure_command("az pipeline \"failed") else {
            panic!("expected pipeline command");
        };
        assert_eq!(rest, "failed");

        let (_, rest) = azure_command("az pr repo:o/p/r \"stale\"").unwrap();
        assert_eq!(split_repository_query(rest), (Some("o/p/r"), "stale"));
    }
}
