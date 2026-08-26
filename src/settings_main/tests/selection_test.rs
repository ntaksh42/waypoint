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

/// 除外した項目に active_item/anchor が向いたままだと、画面上のハイライトは
/// 別の行なのに Edit 等の対象が除外済みの行のまま、という不整合になる
/// (バグ監査で確認済み)。除外時はどちらも動かさないこと。
#[test]
fn toggle_selection_off_does_not_move_active_item_to_the_removed_row() {
    let mut app = app_with_folders(3);
    app.toggle_selection(0);
    app.toggle_selection(1);
    assert_eq!(app.active_item, Some(1));
    assert_eq!(app.selection_anchor, Some(1));

    app.toggle_selection(1);
    assert_eq!(app.selected_items, [0].into_iter().collect());
    assert_eq!(
        app.active_item,
        Some(1),
        "除外した行へ active_item を動かしてはいけない"
    );
    assert_eq!(
        app.selection_anchor,
        Some(1),
        "除外した行へ anchor を動かしてはいけない"
    );
}

/// select_all の直後に anchor が古いままだと、続く Shift+矢印が
/// 全選択ではなく古いアンカーからの小さな範囲へ選択を縮小してしまう
/// (バグ監査で確認済み)。
#[test]
fn select_all_resets_the_anchor_so_shift_arrow_does_not_shrink_the_selection() {
    let mut app = app_with_folders(5);
    app.select_single(2);
    app.select_all();
    assert_eq!(app.selected_items, (0..5).collect());

    app.extend_selection_to(3);
    assert_eq!(
        app.selected_items,
        (0..=3).collect(),
        "select_all 直後の Shift+矢印は先頭からの範囲になるべき"
    );
}
