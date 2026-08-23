use waypoint::config::Item;

use super::{app_with_folders, names};

#[test]
fn remove_selected_removes_every_selected_item_at_once() {
    let mut app = app_with_folders(5);
    app.selected_items = [0, 2, 4].into_iter().collect();
    app.remove_selected();
    assert_eq!(names(&app), vec!["f1", "f3"]);
}

#[test]
fn move_selected_shifts_a_scattered_selection_up_together() {
    let mut app = app_with_folders(5);
    // f0 f1 f2 f3 f4 のうち f1, f3 を選び上へ動かす
    app.selected_items = [1, 3].into_iter().collect();
    app.move_selected(-1);
    assert_eq!(names(&app), vec!["f1", "f0", "f3", "f2", "f4"]);
    assert_eq!(app.selected_items, [0, 2].into_iter().collect());
}

#[test]
fn move_selected_does_nothing_when_any_item_is_already_at_the_edge() {
    let mut app = app_with_folders(4);
    app.selected_items = [0, 2].into_iter().collect();
    app.move_selected(-1);
    // f0 が既に先頭なので全体を動かさない
    assert_eq!(names(&app), vec!["f0", "f1", "f2", "f3"]);
}

#[test]
fn duplicate_selected_inserts_copies_directly_after_each_source() {
    let mut app = app_with_folders(3);
    app.selected_items = [0, 2].into_iter().collect();
    app.duplicate_selected();
    assert_eq!(names(&app), vec!["f0", "f0", "f1", "f2", "f2"]);
}

#[test]
fn copy_and_paste_appends_clones_to_the_end() {
    let mut app = app_with_folders(2);
    app.selected_items = [0].into_iter().collect();
    app.copy_selected();
    app.paste_clipboard();
    assert_eq!(names(&app), vec!["f0", "f1", "f0"]);
}

#[test]
fn move_selected_to_menu_transfers_items_to_the_submenu() {
    let mut app = app_with_folders(3);
    app.config.items.push(Item::Submenu {
        name: "Sub".to_string(),
        items: Vec::new(),
        show_branch: false,
    });
    app.selected_items = [0, 1].into_iter().collect();
    app.move_selected_to_menu(&[3]);
    assert_eq!(names(&app), vec!["f2", "Sub"]);
    let Item::Submenu { items, .. } = &app.config.items[1] else {
        panic!("expected submenu");
    };
    assert_eq!(
        items
            .iter()
            .map(|i| i.label().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec!["f0", "f1"]
    );
}

#[test]
fn move_selected_to_menu_refuses_to_move_into_the_selection_itself() {
    let mut app = app_with_folders(0);
    app.config.items.push(Item::Submenu {
        name: "Sub".to_string(),
        items: Vec::new(),
        show_branch: false,
    });
    app.selected_items = [0].into_iter().collect();
    app.move_selected_to_menu(&[0]);
    // 何も起きない: Sub はまだルートに残ったまま
    assert_eq!(names(&app), vec!["Sub"]);
}
