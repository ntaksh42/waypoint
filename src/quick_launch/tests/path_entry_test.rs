use super::super::path_entry::*;
use crate::quick_launch::Action;

#[test]
fn normalizes_drive_unc_quotes_and_slashes() {
    assert_eq!(
        normalize_path_input(r"C:\Windows").as_deref(),
        Some(r"C:\Windows")
    );
    assert_eq!(
        normalize_path_input("c:/windows").as_deref(),
        Some(r"c:\windows")
    );
    assert_eq!(
        normalize_path_input(r#" "C:\Program Files" "#).as_deref(),
        Some(r"C:\Program Files")
    );
    assert_eq!(
        normalize_path_input(r"\\server\share").as_deref(),
        Some(r"\\server\share")
    );
}

#[test]
fn rejects_non_paths() {
    for query in [
        "",
        "C:",
        "C:foo",
        r"foo\bar",
        "http://example.com",
        r"\",
        r"ps C:\x",
    ] {
        assert!(normalize_path_input(query).is_none(), "{query}");
    }
}

#[test]
fn path_entry_names_last_segment() {
    let entry = path_entry(r"C:\Users\me\").unwrap();
    assert_eq!(entry.name, "me");
    assert_eq!(entry.path, r"C:\Users\me\");
    assert_eq!(entry.breadcrumb, "Path");
    assert!(matches!(entry.action, Action::OpenFolder(_)));
    assert_eq!(path_entry(r"C:\").unwrap().name, r"C:\");
}

#[test]
fn parent_path_stops_at_roots() {
    assert_eq!(parent_path(r"C:\a\b\").as_deref(), Some(r"C:\a\"));
    assert_eq!(parent_path(r"C:\a").as_deref(), Some(r"C:\"));
    assert_eq!(parent_path(r"C:\"), None);
    assert_eq!(
        parent_path(r"\\srv\share\a\b").as_deref(),
        Some(r"\\srv\share\a\")
    );
    assert_eq!(
        parent_path(r"\\srv\share\a\").as_deref(),
        Some(r"\\srv\share\")
    );
    assert_eq!(parent_path(r"\\srv\share\"), None);
    assert_eq!(parent_path(r"\\srv\share"), None);
    assert_eq!(parent_path(r"\\srv"), None);
}

#[test]
fn splits_dir_and_prefix() {
    assert_eq!(split_dir_prefix(r"C:\Prog"), (r"C:\", "Prog"));
    assert_eq!(split_dir_prefix(r"C:\a\"), (r"C:\a\", ""));
}

#[test]
fn completes_with_common_prefix_ignoring_case() {
    let names: Vec<String> = [
        "Program Files",
        "Program Files (x86)",
        "ProgramData",
        "Users",
    ]
    .map(String::from)
    .to_vec();
    let hits = dirs_matching(&names, "prog");
    assert_eq!(hits.len(), 3);
    assert_eq!(common_prefix(&hits), "Program");
    let hits = dirs_matching(&names, "Program Files");
    assert_eq!(common_prefix(&hits), "Program Files");
    assert!(dirs_matching(&names, "zzz").is_empty());
    assert_eq!(common_prefix(&[]), "");
}
