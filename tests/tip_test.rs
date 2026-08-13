//! 土台確認用のテスト。実質的なテストは実装フェーズで追加する。

/// szTip は固定長 UTF-16 配列。長い文字列でも末尾 NUL を残して切り詰めること。
/// (tray::write_tip と同じロジック。private のため検証用に再実装している)
fn write_tip(dst: &mut [u16; 128], text: &str) {
    let src: Vec<u16> = text.encode_utf16().collect();
    let len = src.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&src[..len]);
    dst[len] = 0;
}

#[test]
fn tip_is_nul_terminated() {
    let mut buf = [0u16; 128];
    write_tip(&mut buf, "waypoint");
    assert_eq!(buf[8], 0);
    let text = String::from_utf16_lossy(&buf[..8]);
    assert_eq!(text, "waypoint");
}

#[test]
fn tip_truncates_without_overflow() {
    let mut buf = [0u16; 128];
    write_tip(&mut buf, &"a".repeat(500));
    // 末尾は必ず NUL
    assert_eq!(buf[127], 0);
}
