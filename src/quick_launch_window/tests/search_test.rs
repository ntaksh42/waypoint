use super::super::RowKind;
use super::super::search::{
    accepts_azure_work_item_reply, accepts_everything_reply, build_rows, next_everything_reply_id,
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
