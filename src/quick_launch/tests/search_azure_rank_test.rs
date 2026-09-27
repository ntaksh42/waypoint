//! `az` 検索のノイズ抑制 (breadcrumb / URL は部分一致だけ)、ブランチでの検索、
//! 状態による並び順。

use super::super::azure::AzureIndexed;
use super::super::search::LowerKeys;
use super::super::*;
use super::fixture::index;
use crate::azure_devops::Kind;

struct Pr {
    id: u32,
    title: &'static str,
    repository: &'static str,
    status: &'static str,
    branch: Option<&'static str>,
    is_mine: bool,
}

fn pr(id: u32, title: &'static str) -> Pr {
    Pr {
        id,
        title,
        repository: "app",
        status: "active",
        branch: None,
        is_mine: false,
    }
}

fn index_with(prs: &[Pr]) -> Index {
    let mut index = index();
    index.azure.clear();
    index.azure_work_items.clear();
    index.azure_work_items_lower.clear();
    for pr in prs {
        let url = format!(
            "https://dev.azure.com/org/Waypoint/_git/{}/pullrequest/{}",
            pr.repository, pr.id
        );
        let entry = Entry {
            azure: Some(AzureMeta {
                kind: Kind::PullRequest,
                work_item_type: None,
                status: pr.status.into(),
                is_mine: pr.is_mine,
                needs_my_review: false,
                is_draft: false,
            }),
            name: format!("PR {}: {}", pr.id, pr.title),
            breadcrumb: format!(
                "Azure DevOps — org/Waypoint/{} — {} — by Alice",
                pr.repository, pr.status
            ),
            path: url.clone(),
            action: Action::OpenUrl(url),
            branch: pr.branch.map(str::to_string),
        };
        index.azure.push(AzureIndexed {
            lower: LowerKeys::new(&entry),
            entry,
            kind: Kind::PullRequest,
            status: pr.status.into(),
            is_mine: pr.is_mine,
            is_author: false,
            is_reviewer: false,
            needs_my_review: false,
            waiting_for_others: false,
            is_draft: false,
            ready_to_complete: false,
            is_stale: false,
        });
    }
    index
}

fn names(found: Vec<&Entry>) -> Vec<String> {
    found.into_iter().map(|entry| entry.name.clone()).collect()
}

/// breadcrumb / URL に語の文字が順に散らばって含まれるだけの候補は拾わない。
/// 以前は fuzzy (tier7 / 8) で `profile-xtools` の f…i…x に一致していた。
#[test]
fn letters_scattered_through_the_url_do_not_match() {
    let mut unrelated = pr(7, "Update docs");
    unrelated.repository = "profile-xtools";
    let index = index_with(&[unrelated, pr(8, "Fix crash")]);

    assert_eq!(names(index.search("az pr fix")), ["PR 8: Fix crash"]);
}

#[test]
fn the_common_azure_prefix_matches_nothing() {
    let index = index_with(&[pr(1, "Update docs"), pr(2, "Fix crash")]);

    assert!(index.search("az azure").is_empty());
    assert!(index.search("az devops").is_empty());
}

#[test]
fn author_project_and_repository_still_match_as_substrings() {
    let mut other = pr(2, "Fix crash");
    other.repository = "tools";
    let index = index_with(&[pr(1, "Update docs"), other]);

    assert_eq!(index.search("az alice").len(), 2);
    assert_eq!(index.search("az waypoint").len(), 2);
    assert_eq!(names(index.search("az pr tools")), ["PR 2: Fix crash"]);
}

#[test]
fn source_branch_is_searchable() {
    let mut with_branch = pr(3, "Rework capture");
    with_branch.branch = Some("feature/Hotkey-Capture");
    let index = index_with(&[pr(1, "Update docs"), with_branch]);

    assert_eq!(
        names(index.search("az feature/hotkey")),
        ["PR 3: Rework capture"]
    );
}

#[test]
fn equally_good_matches_put_active_before_closed_and_mine_first() {
    let mut completed = pr(1, "Fix login");
    completed.status = "completed";
    completed.is_mine = true;
    let mut mine = pr(3, "Fix signup");
    mine.is_mine = true;
    let index = index_with(&[completed, pr(2, "Fix logout"), mine]);

    assert_eq!(
        names(index.search("az pr fix")),
        ["PR 3: Fix signup", "PR 2: Fix logout", "PR 1: Fix login"]
    );
}

/// 状態はタイトル一致度より後。完了済みでも語句が完全に一致すれば上に残る。
#[test]
fn title_match_quality_still_beats_state() {
    let mut completed = pr(1, "Payment service rollout");
    completed.status = "completed";
    let index = index_with(&[pr(2, "Service payment cleanup"), completed]);

    assert_eq!(
        names(index.search("az pr payment service")),
        [
            "PR 1: Payment service rollout",
            "PR 2: Service payment cleanup"
        ]
    );
}
