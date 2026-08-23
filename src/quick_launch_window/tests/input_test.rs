use super::super::input::word_start_before;

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
