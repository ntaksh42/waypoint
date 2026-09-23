//! `az ` コマンドの解釈とキャッシュ検索。

use super::super::azure::{AzureCommand, PipelineFilter, PullRequestFilter};
use super::super::*;
use super::fixture::index;

#[test]
fn azure_pr_status_command_filters_cached_pull_requests() {
    let index = index();
    assert_eq!(index.search("az pr active azure").len(), 1);
    assert_eq!(index.search("az pr active mine azure").len(), 1);
    assert_eq!(index.search("az pr active reviewer azure").len(), 1);
    assert_eq!(index.search("az pr active needs-review azure").len(), 1);
    assert!(index.search("az pr active waiting azure").is_empty());
    assert!(index.search("az pr active author azure").is_empty());
    assert!(index.search("az pr completed azure").is_empty());
    assert_eq!(index.search("az wp").len(), 1);
}

#[test]
fn azure_title_search_tolerates_one_transposed_character() {
    let index = index();

    let found = index.search("az pr serach");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "PR 42: Add Azure search");
}

#[test]
fn azure_title_phrase_ranks_before_the_same_words_in_a_different_order() {
    let mut index = index();
    index.azure.clear();
    for name in [
        "PR 10: Service payment cleanup",
        "PR 11: Payment service rollout",
    ] {
        let entry = Entry {
            name: name.into(),
            breadcrumb: "Azure DevOps — org/Waypoint — active".into(),
            path: format!("https://dev.azure.com/org/Waypoint/_git/app/{name}"),
            action: Action::OpenUrl(format!(
                "https://dev.azure.com/org/Waypoint/_git/app/{name}"
            )),
            branch: None,
        };
        index.azure.push(super::super::azure::AzureIndexed {
            lower: super::super::search::LowerKeys::new(&entry),
            entry,
            kind: crate::azure_devops::Kind::PullRequest,
            status: "active".into(),
            is_mine: false,
            is_author: false,
            is_reviewer: false,
            needs_my_review: false,
            waiting_for_others: false,
            is_draft: false,
            ready_to_complete: false,
            is_stale: false,
        });
    }

    let found = index.search("az pr payment service");

    assert_eq!(found[0].name, "PR 11: Payment service rollout");
}

/// PR 番号を指定したら、番号が部分一致するだけの別 PR ではなく、その ID の
/// PR が先頭に来ること。以前は `title.contains(query)` だけを見ていたため
/// `12345` が `PR 123456` / `PR 112345` / 本文に番号を含む PR と同点になり、
/// 以降の並びが実質キャッシュ順で決まって正解が先頭に来なかった。
#[test]
fn azure_pull_request_number_ranks_the_exact_id_first() {
    let mut index = index();
    index.azure.clear();
    for name in [
        "PR 123456: Unrelated rollout",
        "PR 112345: Unrelated cleanup",
        "PR 999: Fix 12345 crash",
        "PR 12345: Add launcher shortcut",
        // 数字の 1 文字違いは別の PR でしかない。タイプミス許容から外す。
        "PR 12845: Typo neighbour",
    ] {
        let url = format!("https://dev.azure.com/org/Waypoint/_git/app/{name}");
        let entry = Entry {
            name: name.into(),
            breadcrumb: "Azure DevOps — org/Waypoint — active".into(),
            path: url.clone(),
            action: Action::OpenUrl(url),
            branch: None,
        };
        index.azure.push(super::super::azure::AzureIndexed {
            lower: super::super::search::LowerKeys::new(&entry),
            entry,
            kind: crate::azure_devops::Kind::PullRequest,
            status: "active".into(),
            is_mine: false,
            is_author: false,
            is_reviewer: false,
            needs_my_review: false,
            waiting_for_others: false,
            is_draft: false,
            ready_to_complete: false,
            is_stale: false,
        });
    }

    let found = index.search("az pr 12345");

    assert_eq!(found[0].name, "PR 12345: Add launcher shortcut");
    assert!(
        !found.iter().any(|entry| entry.name.contains("12845")),
        "数字クエリでタイプミス許容が効いてはいけない: {:?}",
        found.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );
    // 入力途中の部分番号は引き続き拾う (完全一致より下位)。
    let partial = index.search("az pr 1234");
    assert!(partial.len() > 1);
}

#[test]
fn cached_work_items_are_searchable_without_live_api() {
    let mut index = index();
    index.azure_work_items = vec![Entry {
        name: "91: Cache WIT results".into(),
        breadcrumb: "Azure DevOps — org/Waypoint — Bug Active".into(),
        path: "https://dev.azure.com/org/Waypoint/_workitems/edit/91".into(),
        action: Action::OpenUrl("https://dev.azure.com/org/Waypoint/_workitems/edit/91".into()),
        branch: None,
    }];
    index.azure_work_items_lower =
        crate::quick_launch::search::LowerKeys::build_for(&index.azure_work_items);

    let found = index.search_cached_work_items("cache");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "91: Cache WIT results");
    assert!(index.search_cached_work_items("missing").is_empty());

    let all_azure = index.search("az cache");
    assert_eq!(all_azure.len(), 1);
    assert_eq!(all_azure[0].name, "91: Cache WIT results");
}

#[test]
fn azure_command_recognizes_all_supported_subcommands() {
    assert_eq!(
        azure_command("az pr completed done"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Completed,
                status_explicit: true,
                mine: false,
                author: false,
                reviewer: false,
                needs_review: false,
                waiting: false,
                draft: false,
                ready: false,
                stale: false,
                live: false,
            }),
            "done"
        ))
    );
    assert_eq!(
        azure_command("az wit bug"),
        Some((AzureCommand::WorkItems { live: false }, "bug"))
    );
    assert_eq!(
        azure_command("az pipelines release"),
        Some((AzureCommand::Pipelines(PipelineFilter::All), "release"))
    );
    assert_eq!(
        azure_command("az pipeline failed release"),
        Some((AzureCommand::Pipelines(PipelineFilter::Failed), "release"))
    );
    assert_eq!(
        azure_command("az pr active mine launcher"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
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
            }),
            "launcher"
        ))
    );
    // 属性トークンは順不同で並べられる。
    assert_eq!(
        azure_command("az pr mine active launcher"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
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
            }),
            "launcher"
        ))
    );
    // 属性トークンだけで検索語が無ければ空文字列になる。
    assert_eq!(
        azure_command("az pr active"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Active,
                status_explicit: true,
                mine: false,
                author: false,
                reviewer: false,
                needs_review: false,
                waiting: false,
                draft: false,
                ready: false,
                stale: false,
                live: false,
            }),
            ""
        ))
    );
    assert_eq!(
        azure_command("az workitems defect"),
        Some((AzureCommand::WorkItems { live: false }, "defect"))
    );
    assert_eq!(
        azure_command("az platform"),
        Some((AzureCommand::All, "platform"))
    );
    assert_eq!(
        azure_command("az optimize"),
        Some((AzureCommand::Suggest, ""))
    );
}

/// `az <query>` (サブコマンド無し) の `Ctrl+Enter` が Live 検索になること。
/// 以前は `None` を返していたため、Quick Launch が通常の Enter として扱い
/// 選択中の候補をブラウザで開いてしまっていた (検索窓は「Ctrl+Enter Live」
/// と表示しているのに Live にならない)。
#[test]
fn azure_live_request_covers_bare_query() {
    assert_eq!(
        azure_live_request("az waypoint"),
        Some(AzureLiveRequest::AllKinds {
            query: "waypoint".to_string()
        })
    );
    assert_eq!(
        azure_live_request("az wit bug"),
        Some(AzureLiveRequest::WorkItems {
            query: "bug".to_string()
        })
    );
    // 検索対象を持たないコマンドは従来どおり Live 検索の対象外
    assert_eq!(azure_live_request("az optimize"), None);
    assert_eq!(azure_live_request("az project"), None);
}

#[test]
fn azure_prefix_alone_shows_shortcuts_then_all_azure_commands() {
    let index = index();
    let names: Vec<_> = index
        .search("az ")
        .into_iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "az pr",
            "az wit",
            "az pipeline",
            "az project",
            "az optimize"
        ]
    );
}

#[test]
fn incomplete_azure_command_uses_fuzzy_completion() {
    let index = index();
    let found = index.search("az pln");
    assert_eq!(
        found
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["az pipeline"]
    );
    // コマンド候補に当たらない文字列は、従来どおり Azure 全体を検索する。
    assert_eq!(index.search("az wp")[0].name, "PR 42: Add Azure search");
}
