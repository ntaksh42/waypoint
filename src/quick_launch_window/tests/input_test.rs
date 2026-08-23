use super::super::RowKind;
use super::super::input::{first_selectable_row, last_selectable_row, word_start_before};

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn deletes_last_word() {
    let text = to_utf16("hello world test");
    let cursor = text.len();
    assert_eq!(
        word_start_before(&text, cursor),
        to_utf16("hello world ").len()
    );
}

#[test]
fn skips_trailing_spaces_before_word() {
    let text = to_utf16("hello world   ");
    let cursor = text.len();
    assert_eq!(word_start_before(&text, cursor), to_utf16("hello ").len());
}

#[test]
fn stops_at_start_of_text() {
    let text = to_utf16("hello");
    let cursor = text.len();
    assert_eq!(word_start_before(&text, cursor), 0);
}

#[test]
fn cursor_in_middle_of_text() {
    let text = to_utf16("foo bar baz");
    let cursor = to_utf16("foo bar ").len();
    assert_eq!(word_start_before(&text, cursor), to_utf16("foo ").len());
}

#[test]
fn cursor_at_zero_is_noop_boundary() {
    let text = to_utf16("hello");
    assert_eq!(word_start_before(&text, 0), 0);
}

/// End キーは区分見出し行を飛ばして本当の最終項目に着地すること。
/// 見出し行数を差し引かずに `results.len()-1` を直接使うと、見出しの数
/// だけ手前で止まる不具合になる (修正前の実際のバグ)。
#[test]
fn end_key_lands_on_the_last_item_row_skipping_trailing_headers() {
    let rows = [
        RowKind::Header("Folders"),
        RowKind::Item(0),
        RowKind::Item(1),
        RowKind::Header("Apps"),
        RowKind::Item(2),
    ];
    assert_eq!(last_selectable_row(&rows), Some(4));
}

/// Home キーは先頭が見出し行なら次の項目行まで進む
/// (修正前は `select_at(list, 0)` で見出し行自体を選んでしまっていた)。
#[test]
fn home_key_skips_a_leading_header_row() {
    let rows = [
        RowKind::Header("Folders"),
        RowKind::Item(0),
        RowKind::Item(1),
    ];
    assert_eq!(first_selectable_row(&rows), Some(1));
}

#[test]
fn selectable_row_helpers_return_none_for_an_empty_list() {
    assert_eq!(first_selectable_row(&[]), None);
    assert_eq!(last_selectable_row(&[]), None);
}

/// 見出しなしのフラットな一覧では Home/End は単純に先頭・最終行へ着地する。
#[test]
fn home_and_end_on_a_flat_list_without_headers() {
    let rows = [RowKind::Item(0), RowKind::Item(1), RowKind::Item(2)];
    assert_eq!(first_selectable_row(&rows), Some(0));
    assert_eq!(last_selectable_row(&rows), Some(2));
}
