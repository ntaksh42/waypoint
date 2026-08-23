//! DPI・サイズ計算。

use windows::Win32::Foundation::SIZE;

/// 描画寸法に見合うシステムイメージリストを選ぶ。
///
/// 16px のアイコンを 32px へ引き伸ばすと輪郭がにじむ。要求寸法以上で
/// 最も近いリストから取れば、拡大ではなく縮小になり輪郭が保たれる。
pub(crate) fn image_list_for(size: i32) -> i32 {
    use windows::Win32::UI::Shell::{SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL};
    let list = if size <= 16 {
        SHIL_SMALL
    } else if size <= 32 {
        SHIL_LARGE
    } else if size <= 48 {
        SHIL_EXTRALARGE
    } else {
        SHIL_JUMBO
    };
    list as i32
}

/// メニューのアイコン寸法。DPI に追従させる。
///
/// 設定値 (`settings.menu.iconSize`) は 96dpi 基準の論理サイズ。
/// `SM_CXSMICON` は DPI 適用済みの値が返るので、これを基準寸法で
/// 割った比を倍率として使い、設定値へ同じ拡大を掛ける。
pub(crate) fn menu_icon_size() -> SIZE {
    let system = system_small_icon_size();
    let configured = super::ICON_SIZE.with(std::cell::Cell::get);
    if configured == 0 {
        return system;
    }
    let scaled = scale_icon_size(configured, system.cx);
    SIZE {
        cx: scaled,
        cy: scaled,
    }
}

/// システムが定める小アイコンの寸法。DPI 適用済みの値が返る。
fn system_small_icon_size() -> SIZE {
    unsafe {
        let cx = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
        );
        let cy = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSMICON,
        );
        SIZE {
            cx: if cx > 0 { cx } else { BASE_ICON_SIZE },
            cy: if cy > 0 { cy } else { BASE_ICON_SIZE },
        }
    }
}

/// 96dpi でのメニューアイコン寸法。DPI 倍率はこれを基準に求める。
pub const BASE_ICON_SIZE: i32 = 16;

/// 論理サイズへ DPI 倍率を掛ける。倍率は `system / 16`。
///
/// 100% 表示なら設定値がそのまま出る。極端な値でメニューが
/// 壊れないよう上下を留める。
pub fn scale_icon_size(configured: u32, system_cx: i32) -> i32 {
    let configured = configured.clamp(16, 64) as i32;
    let scaled = configured * system_cx.max(BASE_ICON_SIZE) / BASE_ICON_SIZE;
    scaled.clamp(BASE_ICON_SIZE, 256)
}
