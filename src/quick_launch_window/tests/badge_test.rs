use super::super::RowKind;
use super::super::azure_marker::{Fill, Marker, Symbol, marker_for};
use super::super::badge::{
    AzureIconKind, action_tag, action_verb, azure_icon_kind, azure_tile, shows_live_search_hint,
    work_item_style,
};
use super::super::draw_row::section_count;
use crate::azure_devops::WorkItemTypeStyle;
use crate::quick_launch::Action;

#[test]
fn live_search_hint_is_only_shown_in_azure_mode() {
    assert!(shows_live_search_hint(Some("AZURE DEVOPS")));
    assert!(!shows_live_search_hint(Some("FILES")));
    assert!(!shows_live_search_hint(None));
}

#[test]
fn azure_urls_use_distinct_icons_only_in_azure_mode() {
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_git/repo/pullrequest/42"
        ),
        Some(AzureIconKind::PullRequest)
    );
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_workitems/edit/91"
        ),
        Some(AzureIconKind::WorkItem)
    );
    assert_eq!(
        azure_icon_kind(
            Some("AZURE DEVOPS"),
            "https://dev.azure.com/org/project/_build/results?buildId=8"
        ),
        Some(AzureIconKind::Pipeline)
    );
    assert_eq!(
        azure_icon_kind(Some("AZURE DEVOPS"), "https://dev.azure.com/org/project"),
        Some(AzureIconKind::Project)
    );
    assert_eq!(
        azure_icon_kind(Some("BOOKMARKS"), "https://dev.azure.com/org/project"),
        None
    );
}

#[test]
fn action_tags_name_the_candidate_kind() {
    assert_eq!(action_tag(&Action::LaunchApp, None, r"C:\x.lnk"), "App");
    assert_eq!(action_tag(&Action::FocusWindow(1), None, ""), "Window");
    assert_eq!(
        action_tag(&Action::KillProcess(4), Some("KILL"), ""),
        "Kill"
    );
    // `cc ` のフォルダ候補 (path を持つ ReplaceQuery) はフォルダとして見せる
    assert_eq!(
        action_tag(&Action::ReplaceQuery("cc x".into()), None, r"E:\x"),
        "Folder"
    );
    assert_eq!(
        action_tag(&Action::ReplaceQuery("az pr ".into()), None, ""),
        "Command"
    );
}

#[test]
fn azure_candidates_are_tagged_by_url_only_in_azure_mode() {
    let url = "https://dev.azure.com/org/project/_git/repo/pullrequest/42";
    let action = Action::OpenUrl(url.into());
    assert_eq!(action_tag(&action, Some("AZURE DEVOPS"), url), "PR");
    assert_eq!(action_tag(&action, Some("BOOKMARKS"), url), "Link");
    for (url, tag) in [
        (
            "https://dev.azure.com/org/project/_workitems/create/Bug",
            "Work Item",
        ),
        (
            "https://dev.azure.com/org/project/_git/repo/pullrequestcreate",
            "PR",
        ),
    ] {
        assert_eq!(
            action_tag(&Action::OpenUrl(url.into()), Some("AZURE DEVOPS"), url),
            tag
        );
    }
}

#[test]
fn selected_row_verbs_describe_what_enter_does() {
    assert_eq!(action_verb(&Action::FocusWindow(1)), "Switch \u{21B5}");
    assert_eq!(action_verb(&Action::LaunchApp), "Launch \u{21B5}");
    assert_eq!(action_verb(&Action::KillProcess(4)), "Kill \u{21B5}");
    assert_eq!(action_verb(&Action::OpenHelp), "Open \u{21B5}");
}

#[test]
fn section_count_stops_at_the_next_header() {
    let rows = [
        RowKind::Header("Folders"),
        RowKind::Item(0),
        RowKind::Item(1),
        RowKind::Header("Apps"),
        RowKind::Item(2),
    ];
    assert_eq!(section_count(&rows, 0), 2);
    assert_eq!(section_count(&rows, 3), 1);
    assert_eq!(section_count(&[RowKind::Header("Folders")], 0), 0);
}

fn azure_entry(meta: crate::quick_launch::AzureMeta, url: &str) -> crate::quick_launch::Entry {
    crate::quick_launch::Entry {
        azure: Some(meta),
        name: String::new(),
        breadcrumb: String::new(),
        path: url.into(),
        action: Action::OpenUrl(url.into()),
        branch: None,
    }
}

fn meta(
    kind: crate::azure_devops::Kind,
    work_item_type: Option<&str>,
    status: &str,
) -> crate::quick_launch::AzureMeta {
    crate::quick_launch::AzureMeta {
        kind,
        work_item_type: work_item_type.map(str::to_string),
        work_item_style: None,
        status: status.into(),
        is_mine: false,
        needs_my_review: false,
        is_draft: false,
        ready_to_complete: false,
    }
}

const WORK_ITEM_URL: &str = "https://dev.azure.com/org/project/_workitems/edit/1";
const PR_URL: &str = "https://dev.azure.com/org/project/_git/repo/pullrequest/1";

#[test]
fn azure_completions_and_review_shortcuts_have_icons_without_urls() {
    for (query, glyph) in [
        ("az pr ", "⇄"),
        ("az pr active needs-review ", "⇄"),
        ("az wit ", "◆"),
        ("az pipeline ", "▶"),
        ("az project ", "▦"),
    ] {
        let mut entry = azure_entry(meta(crate::azure_devops::Kind::PullRequest, None, ""), "");
        entry.azure = None;
        entry.action = Action::ReplaceQuery(query.into());
        assert_eq!(
            azure_tile(Some("AZURE DEVOPS"), &entry).unwrap().glyph,
            glyph
        );
        assert!(azure_tile(Some("BOOKMARKS"), &entry).is_none());
        entry.action = Action::AzureOptimize;
        assert_eq!(azure_tile(Some("AZURE DEVOPS"), &entry).unwrap().glyph, "★");
    }
}

#[test]
fn work_item_types_get_their_own_colors() {
    use crate::azure_devops::Kind;
    let tile = |work_item_type| {
        azure_tile(
            Some("AZURE DEVOPS"),
            &azure_entry(
                meta(Kind::WorkItem, work_item_type, "Active"),
                WORK_ITEM_URL,
            ),
        )
        .unwrap()
    };
    let bug = tile(Some("Bug"));
    let task = tile(Some("Task"));
    let story = tile(Some("User Story"));
    let unknown = tile(Some("Custom Thing"));
    assert_eq!(bug.glyph, "✱");
    assert_ne!(bug.color, task.color);
    assert_ne!(task.color, story.color);
    // 未知の Type と Type 不明は従来の Work Item の見た目に揃える
    assert_eq!(unknown.color, story.color);
    assert_eq!(tile(None).color, story.color);
}

fn shape(meta: &crate::quick_launch::AzureMeta) -> Option<(Fill, Symbol)> {
    marker_for(meta).map(|Marker { fill, symbol, .. }| (fill, symbol))
}

#[test]
fn state_markers_follow_priority() {
    use crate::azure_devops::Kind;
    let rgb = super::super::rgb;
    let mut review = meta(Kind::PullRequest, None, "active");
    review.needs_my_review = true;
    review.is_draft = true;
    let mut draft = meta(Kind::PullRequest, None, "active");
    draft.is_draft = true;
    let failed = meta(Kind::Pipeline, None, "failed");

    let review = marker_for(&review).unwrap();
    assert_eq!(review.color, rgb(255, 149, 0));
    assert_eq!((review.fill, review.symbol), (Fill::Solid, Symbol::None));
    assert_eq!(shape(&draft), Some((Fill::Hollow, Symbol::None)));
    let failed = marker_for(&failed).unwrap();
    assert_eq!(failed.color, rgb(229, 72, 77));
    assert_eq!(failed.symbol, Symbol::Cross);
    assert_eq!(shape(&meta(Kind::PullRequest, None, "active")), None);
    assert_eq!(shape(&meta(Kind::Project, None, "active")), None);
}

#[test]
fn pull_request_states_have_distinct_shapes() {
    use crate::azure_devops::Kind;
    let pr = |status| meta(Kind::PullRequest, None, status);
    let mut approved = pr("active");
    approved.ready_to_complete = true;
    assert_eq!(shape(&pr("completed")), Some((Fill::Solid, Symbol::Check)));
    assert_eq!(shape(&pr("Abandoned")), Some((Fill::Solid, Symbol::Cross)));
    assert_eq!(shape(&approved), Some((Fill::Hollow, Symbol::Check)));
    // Draft は承認済みより優先する
    approved.is_draft = true;
    assert_eq!(shape(&approved), Some((Fill::Hollow, Symbol::None)));
}

#[test]
fn pipeline_states_have_distinct_shapes() {
    use crate::azure_devops::Kind;
    let pipeline = |status| shape(&meta(Kind::Pipeline, None, status));
    let shapes = [
        pipeline("succeeded"),
        pipeline("failed"),
        pipeline("inProgress"),
        pipeline("canceled"),
        pipeline("partiallySucceeded"),
    ];
    assert!(shapes.iter().all(Option::is_some));
    for (i, a) in shapes.iter().enumerate() {
        assert!(shapes[i + 1..].iter().all(|b| a != b));
    }
    // 定義・未開始・未知の状態は点を出さない
    assert_eq!(pipeline("definition"), None);
    assert_eq!(pipeline("notStarted"), None);
}

#[test]
fn work_item_states_map_across_process_templates() {
    use crate::azure_devops::Kind;
    let state = |status| shape(&meta(Kind::WorkItem, Some("Task"), status));
    let new = Some((Fill::Hollow, Symbol::None));
    let active = Some((Fill::Solid, Symbol::None));
    let resolved = Some((Fill::Half, Symbol::None));
    let closed = Some((Fill::Solid, Symbol::Check));
    for status in ["New", "Proposed", "To Do"] {
        assert_eq!(state(status), new, "{status}");
    }
    for status in ["Active", "Committed", "Doing"] {
        assert_eq!(state(status), active, "{status}");
    }
    assert_eq!(state("Resolved"), resolved);
    for status in ["Closed", "Done"] {
        assert_eq!(state(status), closed, "{status}");
    }
    assert_eq!(state("Removed"), Some((Fill::Solid, Symbol::Cross)));
    assert_eq!(state("Whatever"), None);
}

#[test]
fn closed_items_are_dimmed_and_still_show_their_outcome() {
    use crate::azure_devops::Kind;
    let completed = azure_tile(
        Some("AZURE DEVOPS"),
        &azure_entry(meta(Kind::PullRequest, None, "completed"), PR_URL),
    )
    .unwrap();
    assert!(completed.closed);
    assert_eq!(completed.marker.unwrap().symbol, Symbol::Check);
    let abandoned = azure_tile(
        Some("AZURE DEVOPS"),
        &azure_entry(meta(Kind::PullRequest, None, "abandoned"), PR_URL),
    )
    .unwrap();
    assert!(abandoned.closed);
    assert_eq!(abandoned.marker.unwrap().symbol, Symbol::Cross);
}

#[test]
fn azure_tiles_only_appear_in_azure_mode_and_fall_back_to_the_url() {
    let with_meta = azure_entry(
        meta(crate::azure_devops::Kind::PullRequest, None, "active"),
        PR_URL,
    );
    assert!(azure_tile(Some("BOOKMARKS"), &with_meta).is_none());

    let mut without_meta = with_meta.clone();
    without_meta.azure = None;
    let tile = azure_tile(Some("AZURE DEVOPS"), &without_meta).unwrap();
    assert_eq!(tile.glyph, "⇄");
    assert!(tile.marker.is_none() && !tile.closed);
}

fn api_style(color: (u8, u8, u8), icon: Option<&str>) -> WorkItemTypeStyle {
    WorkItemTypeStyle {
        color,
        icon: icon.map(str::to_string),
    }
}

#[test]
fn work_item_style_prefers_the_api_color_and_falls_back_without_it() {
    let rgb = super::super::rgb;
    // API の色が最優先 (Bug の既定の赤とは違う色でも API を採る)
    let (color, glyph) = work_item_style("Bug", Some(&api_style((1, 2, 3), Some("icon_insect"))));
    assert_eq!(color, rgb(1, 2, 3));
    assert_eq!(glyph, "✱");
    // API が無ければ従来の Type 名による配色、未知の Type は青の ◆
    assert_eq!(work_item_style("Bug", None), (rgb(204, 41, 61), "✱".into()));
    assert_eq!(
        work_item_style("Product Backlog Item", None),
        (rgb(0, 120, 212), "◆".into())
    );
}

#[test]
fn work_item_glyph_falls_back_from_icon_id_to_type_name_to_initial() {
    let rgb = super::super::rgb;
    // アイコン ID が組込みに無ければ Type 名の既定の記号
    let unknown_icon = api_style((9, 9, 9), Some("icon_unheard_of"));
    assert_eq!(work_item_style("Task", Some(&unknown_icon)).1, "✓");
    // Type 名にも既定が無ければ頭文字
    assert_eq!(
        work_item_style("product backlog item", Some(&unknown_icon)),
        (rgb(9, 9, 9), "P".into())
    );
    assert_eq!(
        work_item_style("Risk", Some(&api_style((9, 9, 9), None))).1,
        "R"
    );
}

#[test]
fn work_item_tile_uses_the_style_carried_by_the_meta() {
    use crate::azure_devops::Kind;
    let mut with_style = meta(Kind::WorkItem, Some("Impediment"), "Active");
    with_style.work_item_style = Some(api_style((10, 20, 30), Some("icon_flame")));
    let tile = azure_tile(
        Some("AZURE DEVOPS"),
        &azure_entry(with_style, WORK_ITEM_URL),
    )
    .unwrap();
    assert_eq!(tile.color, super::super::rgb(10, 20, 30));
    assert_eq!(tile.glyph, "⚠");
}
