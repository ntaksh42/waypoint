use super::super::draw_footer::{footer_hints, footer_position};
use super::super::draw_row::{detail_primary, detail_secondary, display_path};
use crate::config::OpenMode;
use crate::quick_launch::{Action, Entry};

fn entry(breadcrumb: &str, path: &str, action: Action) -> Entry {
    Entry {
        name: "x".into(),
        breadcrumb: breadcrumb.into(),
        path: path.into(),
        action,
        branch: None,
    }
}

#[test]
fn breadcrumb_that_is_a_prefix_of_the_path_is_not_repeated() {
    let folder = entry(
        r"E:\",
        r"E:\waypoint",
        Action::OpenFolder(OpenMode::NewWindow),
    );
    assert_eq!(detail_primary(&folder), r"E:\waypoint");
    assert_eq!(detail_secondary(&folder), None);
    let same = entry(r"E:\", r"E:\", Action::OpenFolder(OpenMode::NewWindow));
    assert_eq!(detail_secondary(&same), None);
}

#[test]
fn menu_breadcrumb_is_kept_next_to_the_path() {
    let folder = entry(
        "My Special Folders",
        r"C:\Users\me\Desktop",
        Action::OpenFolder(OpenMode::NewWindow),
    );
    assert_eq!(detail_primary(&folder), "My Special Folders");
    assert_eq!(
        detail_secondary(&folder).as_deref(),
        Some("  \u{00B7}  C:\\Users\\me\\Desktop")
    );
}

#[test]
fn open_window_detail_shows_only_the_process_name() {
    let window = entry("Open Windows — claude.exe", "", Action::FocusWindow(1));
    assert_eq!(detail_primary(&window), "claude.exe");
    assert_eq!(detail_secondary(&window), None);
}

#[test]
fn footer_hints_follow_the_selected_candidate() {
    let window = entry("", "", Action::FocusWindow(1));
    assert_eq!(
        footer_hints(Some(&window)),
        [("\u{21B5}", "Switch"), ("Ctrl+W", "Close window")]
    );
    let folder = entry("", r"E:\x", Action::OpenFolder(OpenMode::NewWindow));
    let keys: Vec<_> = footer_hints(Some(&folder)).iter().map(|h| h.0).collect();
    assert_eq!(
        keys,
        [
            "\u{21B5}",
            "Shift+\u{21B5}",
            "Ctrl+E",
            "Ctrl+C",
            "Ctrl+Shift+\u{21B5}"
        ]
    );
    assert!(footer_hints(None).is_empty());
}

#[test]
fn footer_position_counts_items_from_one() {
    assert_eq!(footer_position(Some(2), 24).as_deref(), Some("3 / 24"));
    assert_eq!(footer_position(None, 5).as_deref(), Some("5 results"));
    assert_eq!(footer_position(None, 0), None);
}

#[test]
fn start_menu_paths_are_shortened_for_display() {
    assert_eq!(
        display_path(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Tools\VS.lnk"),
        r"Start Menu\Tools\VS.lnk"
    );
    assert_eq!(display_path(r"E:\waypoint"), r"E:\waypoint");
}
