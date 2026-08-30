use super::super::layout::rows_height;
use super::super::{HEADER_HEIGHT, ROW_HEIGHT, RowKind};

#[test]
fn rows_height_sums_item_rows_at_row_height() {
    let rows = vec![RowKind::Item(0), RowKind::Item(1), RowKind::Item(2)];
    assert_eq!(rows_height(&rows, 24), ROW_HEIGHT * 3);
}

#[test]
fn rows_height_uses_header_height_for_header_rows() {
    let rows = vec![RowKind::Header("Folders"), RowKind::Item(0)];
    assert_eq!(rows_height(&rows, 24), HEADER_HEIGHT + ROW_HEIGHT);
}

#[test]
fn rows_height_is_capped_by_max_rows_so_it_matches_the_visible_scroll_window() {
    let rows: Vec<RowKind> = (0..30).map(RowKind::Item).collect();
    assert_eq!(rows_height(&rows, 24), ROW_HEIGHT * 24);
}

#[test]
fn rows_height_never_collapses_to_zero_while_results_are_pending() {
    assert_eq!(rows_height(&[], 24), ROW_HEIGHT);
}
