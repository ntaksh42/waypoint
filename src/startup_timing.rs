//! 通常起動でだけ有効にする、起動区間の診断ログ。
//!
//! GUI サブシステムでは標準出力を使えないため、`--startup-timing` を付けた
//! 起動だけ既存のログへ相対時間を書き出す。通常の表示・検索経路では何もしない。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

static STARTED: OnceLock<Instant> = OnceLock::new();

/// 起動計測を有効にする。同一プロセス内では最初の呼び出しだけが効く。
pub fn enable() {
    let _ = STARTED.set(Instant::now());
}

pub fn enabled() -> bool {
    STARTED.get().is_some()
}

/// 計測開始からの経過時間付きで、診断ログへ 1 行書く。
pub fn mark(label: &str) {
    let Some(started) = STARTED.get() else {
        return;
    };
    crate::panic_log::record(&format_entry(started.elapsed(), label));
}

/// 個別区間の所要時間と、その完了時点を記録する。
pub fn mark_elapsed(label: &str, elapsed: Duration) {
    let Some(started) = STARTED.get() else {
        return;
    };
    crate::panic_log::record(&format_entry(
        started.elapsed(),
        &format!("{label} ({:.1} ms)", elapsed.as_secs_f64() * 1000.0),
    ));
}

fn format_entry(elapsed: Duration, label: &str) -> String {
    format!(
        "startup timing +{:.1} ms: {label}",
        elapsed.as_secs_f64() * 1000.0
    )
}

#[cfg(test)]
mod tests {
    use super::format_entry;
    use std::time::Duration;

    #[test]
    fn formats_elapsed_milliseconds() {
        assert_eq!(
            format_entry(Duration::from_micros(1_250), "config loaded"),
            "startup timing +1.2 ms: config loaded"
        );
    }
}
