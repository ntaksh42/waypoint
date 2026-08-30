use super::super::RowKind;
use super::super::search::{
    accepts_azure_work_item_reply, accepts_everything_reply, build_rows, next_everything_reply_id,
    refined_search_term,
};
use crate::config::OpenMode;
use crate::quick_launch::{Action, Entry};

fn folder_entry(name: &str) -> Entry {
    Entry {
        name: name.to_string(),
        breadcrumb: String::new(),
        path: format!(r"C:\{name}"),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }
}

#[test]
fn stale_everything_reply_is_rejected_after_a_new_query() {
    let first = next_everything_reply_id(0);
    let second = next_everything_reply_id(first);

    assert!(!accepts_everything_reply(true, second, first));
    assert!(accepts_everything_reply(true, second, second));
    assert!(!accepts_everything_reply(false, second, second));
}

#[test]
fn stale_azure_work_item_request_is_rejected_after_more_typing() {
    assert!(!accepts_azure_work_item_reply(true, 8, 7));
    assert!(!accepts_azure_work_item_reply(false, 8, 8));
    assert!(accepts_azure_work_item_reply(true, 8, 8));
}

#[test]
fn refined_search_only_reuses_candidates_for_a_narrower_local_query() {
    assert_eq!(refined_search_term("way", "wayp"), Some("wayp"));
    assert_eq!(refined_search_term("b git", "b gith"), Some("gith"));
    assert_eq!(refined_search_term("b git", "h git"), None);
    assert_eq!(refined_search_term("way", "way"), None);
    assert_eq!(refined_search_term("", "way"), None);
    assert_eq!(refined_search_term("f read", "f readm"), None);
    assert_eq!(refined_search_term("az pr", "az pra"), None);
    // 通常検索中に `ps ` を打ち足してターミナル検索モードへ入った直後は、
    // 通常検索の結果 (フォルダ以外を含む) を絞り込むのではなく、
    // Index::search の `ps ` 専用索引を新たに引き直す必要がある。
    // `local_search_scope` が "ps " を認識せず両方 "normal" 扱いになると、
    // ここが誤って Some を返し、フォルダ限定の絞り込みが効かなくなる
    // (実機で `ps` から `ps waypoint` と打った際に再現)。
    assert_eq!(refined_search_term("ps", "ps waypoint"), None);
    assert_eq!(refined_search_term("ps way", "ps wayp"), Some("wayp"));
}

/// リストへ流し込む上限は、ウィンドウに実際に映る行数 (`visible_results`
/// の上限 24) 以上あれば足り、それを超えて作った行は画面に出ないまま
/// 1 打鍵ごとの `LB_ADDSTRING` と `WM_MEASUREITEM` を増やすだけになる
/// (`az wit` のキャッシュ 300 件規模でカクつきとして表面化した)。
/// 両者がずれると無駄が再発するので、ここで結び付けておく。
#[test]
fn list_result_cap_matches_the_largest_visible_row_count() {
    assert_eq!(super::super::MAX_LIST_RESULTS, 24);
    // Everything も同じ枠で先に絞っている (要求段階での上限)
    assert_eq!(
        super::super::EVERYTHING_MAX_RESULTS as usize,
        super::super::MAX_LIST_RESULTS
    );
}

#[test]
fn build_rows_without_headers_is_a_flat_one_to_one_mapping() {
    let results = vec![folder_entry("a"), folder_entry("b")];
    let (labels, rows) = build_rows(&results, &[]);
    assert_eq!(labels.len(), 2);
    assert!(matches!(
        rows.as_slice(),
        [RowKind::Item(0), RowKind::Item(1)]
    ));
}

#[test]
fn build_rows_inserts_a_header_row_before_each_section_start() {
    let results = vec![folder_entry("a"), folder_entry("b"), folder_entry("c")];
    // "Folders" は results[0] の直前、"Apps" は results[2] の直前に挿入される想定
    let section_headers = [(0, "Folders"), (2, "Apps")];
    let (labels, rows) = build_rows(&results, &section_headers);
    assert_eq!(labels.len(), 5);
    assert!(matches!(
        rows.as_slice(),
        [
            RowKind::Header("Folders"),
            RowKind::Item(0),
            RowKind::Item(1),
            RowKind::Header("Apps"),
            RowKind::Item(2),
        ]
    ));
}
