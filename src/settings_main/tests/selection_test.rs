use super::app_with_folders;

#[test]
fn extend_selection_to_covers_the_range_from_the_anchor() {
    let mut app = app_with_folders(5);
    app.select_single(1);
    app.extend_selection_to(3);
    assert_eq!(app.selected_items, [1, 2, 3].into_iter().collect());

    // アンカーは 1 のまま。逆方向へ伸ばすと 1 側の選択に切り替わる
    app.extend_selection_to(0);
    assert_eq!(app.selected_items, [0, 1].into_iter().collect());
}

#[test]
fn toggle_selection_adds_and_removes_a_single_item() {
    let mut app = app_with_folders(3);
    app.select_single(0);
    app.toggle_selection(2);
    assert_eq!(app.selected_items, [0, 2].into_iter().collect());
    app.toggle_selection(0);
    assert_eq!(app.selected_items, [2].into_iter().collect());
}
