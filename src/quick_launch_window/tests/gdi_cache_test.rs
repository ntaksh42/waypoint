//! 固定色 GDI オブジェクトの使い回し。
//!
//! 描画そのもの (色や位置が合っているか) はここでは判定できない。
//! ここで守りたいのは「同じ色なら同じハンドルが返る」= 行ごとに
//! `CreateSolidBrush` / `DeleteObject` が走らないこと。

use super::super::gdi_cache;
use super::super::rgb;

#[test]
fn same_color_returns_the_same_brush_handle() {
    // 2 回目以降が同じハンドルなら、行ごとの生成・破棄が消えている。
    let first = gdi_cache::background_brush();
    let second = gdi_cache::background_brush();
    assert_eq!(first.0, second.0);
    assert!(!first.0.is_null(), "ブラシの生成に失敗している");
}

#[test]
fn same_color_returns_the_same_pen_handle() {
    let first = gdi_cache::card_pen();
    let second = gdi_cache::card_pen();
    assert_eq!(first.0, second.0);
    assert!(!first.0.is_null(), "ペンの生成に失敗している");
}

#[test]
fn color_keyed_table_reuses_per_color() {
    let red = rgb(200, 40, 40);
    let blue = rgb(40, 80, 200);

    let red_first = gdi_cache::brush_for(red);
    let blue_first = gdi_cache::brush_for(blue);
    let red_second = gdi_cache::brush_for(red);
    let blue_second = gdi_cache::brush_for(blue);

    assert_eq!(red_first.0, red_second.0, "同じ色は使い回す");
    assert_eq!(blue_first.0, blue_second.0, "同じ色は使い回す");
    assert_ne!(red_first.0, blue_first.0, "違う色は別のハンドル");
}

#[test]
fn color_keyed_pens_reuse_per_color() {
    let green = rgb(40, 200, 90);
    let first = gdi_cache::pen_for(green);
    let second = gdi_cache::pen_for(green);
    assert_eq!(first.0, second.0);
}

/// 固定スロットと色キー表は別の入れ物なので、同じ色でもハンドルは
/// 一致しなくてよい。ここで確かめたいのは、どちらも有効なハンドルを
/// 返すこと (どちらかが 0 を返すと描画が黙って消える)。
#[test]
fn fixed_slots_and_color_table_both_return_valid_handles() {
    assert!(!gdi_cache::surface_hover_brush().0.is_null());
    assert!(!gdi_cache::surface_pen().0.is_null());
    assert!(!gdi_cache::accent_pen().0.is_null());
    assert!(!gdi_cache::brush_for(rgb(1, 2, 3)).0.is_null());
    assert!(!gdi_cache::pen_for(rgb(3, 2, 1)).0.is_null());
}
