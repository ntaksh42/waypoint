use super::super::State;
use super::super::path_browse::{PathList, apply_results, plan_path_tab};
use crate::quick_launch::path_entry;

fn state_with_list(dir: &str, names: &[&str]) -> State {
    State {
        path_list: Some(PathList {
            dir: dir.to_string(),
            names: names.iter().map(|s| s.to_string()).collect(),
        }),
        ..State::default()
    }
}

#[test]
fn tab_without_listing_waits_for_it() {
    let mut state = State::default();
    assert_eq!(plan_path_tab(&mut state, r"C:\Prog", false), None);
    assert_eq!(state.path_tab_pending.as_deref(), Some(r"C:\Prog"));
    assert!(state.path_browse);
}

#[test]
fn tab_completes_unique_match_and_descends() {
    let mut state = state_with_list(r"C:\", &["Program Files", "Users"]);
    assert_eq!(
        plan_path_tab(&mut state, r"c:\prog", false).as_deref(),
        Some(r"c:\Program Files\")
    );
    assert!(state.path_cycle.is_none());
}

#[test]
fn tab_extends_to_common_prefix_then_cycles() {
    let mut state = state_with_list(
        r"C:\",
        &["Program Files", "Program Files (x86)", "ProgramData"],
    );
    assert_eq!(
        plan_path_tab(&mut state, r"C:\pr", false).as_deref(),
        Some(r"C:\Program")
    );
    // 共通接頭辞まで進んだ後は、押すたびに候補を巡る
    assert_eq!(
        plan_path_tab(&mut state, r"C:\Program", false).as_deref(),
        Some(r"C:\Program Files")
    );
    assert_eq!(
        plan_path_tab(&mut state, r"C:\Program Files", false).as_deref(),
        Some(r"C:\Program Files (x86)")
    );
    // Shift+Tab は逆順
    assert_eq!(
        plan_path_tab(&mut state, r"C:\Program Files (x86)", true).as_deref(),
        Some(r"C:\Program Files")
    );
}

#[test]
fn tab_after_backslash_lists_children_without_changing_text() {
    let mut state = state_with_list(r"C:\", &["a", "b"]);
    assert_eq!(
        plan_path_tab(&mut state, r"C:\", false).as_deref(),
        Some(r"C:\")
    );
    let cycle = state.path_cycle.as_ref().unwrap();
    assert_eq!(cycle.matches, ["a", "b"]);
    assert_eq!(cycle.index, None);
    assert_eq!(
        plan_path_tab(&mut state, r"C:\", true).as_deref(),
        Some(r"C:\b")
    );
}

#[test]
fn shift_tab_goes_up_and_stops_at_root() {
    let mut state = State::default();
    assert_eq!(
        plan_path_tab(&mut state, r"C:\a\b\", true).as_deref(),
        Some(r"C:\a\")
    );
    assert_eq!(plan_path_tab(&mut state, r"C:\", true), None);
    assert_eq!(plan_path_tab(&mut state, r"\\srv\share\", true), None);
}

#[test]
fn browsing_replaces_results_with_subfolders() {
    let mut state = state_with_list(r"C:\", &["Program Files", "Users"]);
    state.path_browse = true;
    apply_results(&mut state, path_entry(r"C:\U").unwrap());
    let names: Vec<_> = state.results.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["U", "Users"]);
    assert_eq!(state.results[1].path, r"C:\Users");
}

#[test]
fn without_browsing_path_entry_is_prepended() {
    let mut state = state_with_list(r"C:\", &["Users"]);
    apply_results(&mut state, path_entry(r"C:\U").unwrap());
    assert_eq!(state.results.len(), 1);
    assert_eq!(state.results[0].path, r"C:\U");
}
