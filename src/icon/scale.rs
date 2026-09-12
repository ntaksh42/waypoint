//! システムイメージリストの選択。

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
