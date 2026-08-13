//! ログ出力 (06 章) の確認。
//!
//! 実ファイル (`%APPDATA%\waypoint\log.txt`) へ追記する作りなので、
//! テストでは既存の内容を壊さないよう、書き込み前後の差分だけを見る。

fn read_log() -> String {
    waypoint::panic_log::log_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

#[test]
fn record_appends_a_timestamped_line() {
    let before = read_log().len();
    let marker = format!("test marker {}", std::process::id());
    waypoint::panic_log::record(&marker);

    let after = read_log();
    assert!(after.len() > before, "ログが伸びていない");

    let appended = &after[before..];
    assert!(appended.contains(&marker), "記録されていない: {appended:?}");
    assert!(
        appended.starts_with('['),
        "時刻が付いていない: {appended:?}"
    );
    assert!(appended.ends_with('\n'), "改行で終わっていない");
}

/// `extern "system"` の中で panic しても abort させないための
/// `catch_unwind` が期待通り捕まえること。window_proc と同じ形。
#[test]
fn catch_unwind_recovers_from_panic() {
    let result = std::panic::catch_unwind(|| {
        panic!("simulated window_proc panic");
    });
    assert!(result.is_err(), "panic を捕まえられていない");
    // 捕まえた後も処理を続けられる
    let after = std::panic::catch_unwind(|| 42);
    assert_eq!(after.ok(), Some(42));
}
