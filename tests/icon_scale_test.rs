//! メニューアイコンの DPI 倍率計算のテスト。

use waypoint::icon::{BASE_ICON_SIZE, scale_icon_size};

/// 100% 表示 (SM_CXSMICON が 16) では設定値がそのまま出る。
#[test]
fn uses_the_configured_size_at_100_percent() {
    assert_eq!(scale_icon_size(32, BASE_ICON_SIZE), 32);
    assert_eq!(scale_icon_size(16, BASE_ICON_SIZE), 16);
}

/// 150% 表示 (SM_CXSMICON が 24) では設定値も 1.5 倍になる。
#[test]
fn scales_with_the_system_metric() {
    assert_eq!(scale_icon_size(32, 24), 48);
}

/// 設定値が範囲外でもメニューが壊れる寸法にはしない。
#[test]
fn keeps_out_of_range_values_usable() {
    assert_eq!(scale_icon_size(0, BASE_ICON_SIZE), 16);
    assert_eq!(scale_icon_size(4096, BASE_ICON_SIZE), 64);
}
