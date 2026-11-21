use super::super::azure_live::{
    filtered_live_message, missing_azure_work_item_id, work_item_lookup_entry,
};
use crate::azure_devops::Kind;
use crate::quick_launch::{Action, AzureMeta, Entry, WorkItemFilter};

fn entry(name: &str, kind: Option<Kind>) -> Entry {
    Entry {
        azure: kind.map(|kind| AzureMeta {
            kind,
            work_item_type: None,
            work_item_style: None,
            status: String::new(),
            is_mine: false,
            needs_my_review: false,
            is_draft: false,
            ready_to_complete: false,
        }),
        name: name.into(),
        breadcrumb: String::new(),
        path: String::new(),
        action: Action::OpenUrl(String::new()),
        branch: None,
    }
}

/// `az <番号>` で同じ番号の PR が居ても、Work Item が無ければ Work Item の入口を出す。
#[test]
fn cross_search_by_number_only_counts_work_items_as_a_work_item_hit() {
    assert!(missing_azure_work_item_id(
        &[entry("PR 68: Add launcher", Some(Kind::PullRequest))],
        "68"
    ));
    assert!(!missing_azure_work_item_id(
        &[entry("68: Cache WIT results", Some(Kind::WorkItem))],
        "68"
    ));
    assert!(!missing_azure_work_item_id(&[], "rollout"));
}

/// 入口は `#<番号>` で Live 検索へ渡し、ID 完全一致の取得にする。
#[test]
fn work_item_lookup_entry_fetches_by_exact_id() {
    let lookup = work_item_lookup_entry(" 68 ");
    assert_eq!(lookup.name, "Look up Work Item 68 on Azure DevOps");
    assert_eq!(
        lookup.action,
        Action::AzureLiveWorkItemSearch {
            filter: WorkItemFilter::default(),
            query: "#68".to_string(),
        }
    );
}

#[test]
fn live_message_explains_when_attributes_drop_every_fetched_item() {
    let range = Some("Live work items: up to 300 per project.".to_string());
    assert_eq!(filtered_live_message(5, 2, range.clone()), range);
    assert_eq!(filtered_live_message(0, 0, range.clone()), range);
    let message = filtered_live_message(5, 0, range).unwrap();
    assert!(message.starts_with("None of the 5 fetched work items match the attributes."));
    assert!(message.contains("up to 300"));
}
