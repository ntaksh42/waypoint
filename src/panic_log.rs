//! ログ出力 (06 章のログ要件: 起動・設定読込・エラーのみ記録) 。
//!
//! GUI サブシステム (`windows_subsystem = "windows"`) では stderr が
//! どこにも繋がらないため、既定の panic ハンドラの出力は消える。
//! `window_proc` のような `extern "system"` の中で panic すると
//! 「unwind できない関数での panic」として即 abort するので、
//! 画面には何も出ないままプロセスだけが消える。原因を後から
//! 追えるように、パニックの内容をファイルへ追記する。
//!
//! panic 以外の記録もここへ集約する。出力先が同じで、
//! 「後から原因を追う」という目的も同じため。

use std::io::Write;
use std::path::PathBuf;

/// ログの出力先。`%APPDATA%\waypoint\log.txt`
pub fn log_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("waypoint").join("log.txt"))
}

/// panic フックを登録する。`main` の最初に一度だけ呼ぶ。
pub fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_entry(&format_entry(info));
        // 既定のフックも呼び、デバッグ実行時は stderr にも出す
        default_hook(info);
    }));
}

/// 1 行記録する。起動・設定読込・エラーのみに使う (06 章) 。
///
/// 頻繁に呼ぶ用途は想定しない。メニュー表示経路からは呼ばないこと
/// (ファイル I/O が表示遅延に乗る)。
pub fn record(message: &str) {
    write_entry(&format!("[{}] {message}\n", timestamp()));
}

fn format_entry(info: &std::panic::PanicHookInfo<'_>) -> String {
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());
    // ペイロードは &str か String のことが多い。それ以外は型名すら取れない
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());

    format!("[{}] panic at {location}: {message}\n", timestamp())
}

/// 追記で書く。ログのために起動を失敗させないよう、エラーは握り潰す。
fn write_entry(text: &str) {
    let Some(path) = log_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(text.as_bytes());
    }
}

/// UNIX 秒。時刻整形のためだけに依存を増やしたくないので生の秒で残す。
fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_is_under_appdata_waypoint() {
        let path = log_path().expect("APPDATA を解決できること");
        assert!(path.ends_with("waypoint\\log.txt"), "{path:?}");
    }

    /// パニックの位置とメッセージが 1 行に入ること。
    #[test]
    fn formats_message_and_location() {
        let entry = std::panic::catch_unwind(|| {
            let previous = std::panic::take_hook();
            let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let sink = captured.clone();
            std::panic::set_hook(Box::new(move |info| {
                *sink.lock().unwrap() = format_entry(info);
            }));
            let _ = std::panic::catch_unwind(|| panic!("boom"));
            std::panic::set_hook(previous);
            captured.lock().unwrap().clone()
        })
        .expect("フック登録自体は失敗しないこと");

        assert!(entry.contains("boom"), "{entry}");
        assert!(entry.contains("panic_log.rs"), "{entry}");
        assert!(entry.ends_with('\n'), "{entry}");
    }
}
