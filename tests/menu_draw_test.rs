//! メニューのオーナードロー描画で使うダーク判定のテスト。

use waypoint::menu_draw::is_light_text;

/// 実測値。ダーク指定では白、ライトでは黒が返る。
#[test]
fn detects_dark_theme_from_text_color() {
    assert!(is_light_text(0x00FFFFFF), "白文字ならダーク");
    assert!(!is_light_text(0x00000000), "黒文字ならライト");
}

/// 判定に使うのは通常項目の文字色だけ。中間色でも極端に振れない。
#[test]
fn mid_gray_does_not_flip_on_a_hair() {
    // 128 前後は「暗い」側に倒す。閾値は 0x180 = 各色 128 相当
    assert!(!is_light_text(0x00808080));
    assert!(is_light_text(0x00C0C0C0));
}
