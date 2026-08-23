use waypoint::config::{Config, Item};

use super::super::drafts::ItemDraft;
use super::super::helpers::{items_at, items_at_mut};
use super::super::ui_items::reorder_target;

#[test]
fn nested_item_path_resolves_submenu() {
    let mut config = Config {
        items: vec![Item::Submenu {
            name: "Tools".to_string(),
            items: vec![Item::Separator { name: None }],
            show_branch: false,
        }],
        ..Default::default()
    };

    items_at_mut(&mut config, &[0])
        .unwrap()
        .push(Item::Separator {
            name: Some("Build".to_string()),
        });

    assert_eq!(items_at(&config, &[0]).unwrap().len(), 2);
}

#[test]
fn editing_submenu_preserves_its_children() {
    let original = Item::Submenu {
        name: "Old".to_string(),
        items: vec![Item::Separator { name: None }],
        show_branch: false,
    };
    let mut draft = ItemDraft::from_item(0, &original);
    draft.name = "New".to_string();

    assert_eq!(
        draft.into_item(),
        Item::Submenu {
            name: "New".to_string(),
            items: vec![Item::Separator { name: None }],
            show_branch: false,
        }
    );
}

/// 実際に remove → insert したときの並びを返す。
/// `reorder_target` の補正が正しいことを、結果の並びで確かめる。
fn reordered(len: usize, from: usize, insert_at: usize) -> Vec<usize> {
    let mut items: Vec<usize> = (0..len).collect();
    let item = items.remove(from);
    items.insert(reorder_target(from, insert_at), item);
    items
}

/// 後ろへ動かすとき、取り除きで添字が 1 つ詰まる分を補正する。
#[test]
fn dragging_down_lands_before_the_target() {
    // [0,1,2,3,4] の 1 を「3 の手前」へ → [0,2,1,3,4]
    assert_eq!(reordered(5, 1, 3), vec![0, 2, 1, 3, 4]);
}

/// 前へ動かすときは補正しない。
#[test]
fn dragging_up_lands_at_the_target() {
    // [0,1,2,3,4] の 3 を「1 の手前」へ → [0,3,1,2,4]
    assert_eq!(reordered(5, 3, 1), vec![0, 3, 1, 2, 4]);
}

/// 一覧の末尾へ落とすと最後尾に来る。
#[test]
fn dragging_to_the_tail_lands_last() {
    assert_eq!(reordered(4, 0, 4), vec![1, 2, 3, 0]);
}

/// 自分の位置とその直後は、どちらも並びを変えない。
#[test]
fn dropping_onto_itself_keeps_the_order() {
    assert_eq!(reordered(4, 2, 2), vec![0, 1, 2, 3]);
    assert_eq!(reordered(4, 2, 3), vec![0, 1, 2, 3]);
}
