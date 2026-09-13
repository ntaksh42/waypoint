//! Web 検索 (`??` プレフィックス、FR-9.21)。
//!
//! 検索語を URL へ組み立てるだけのモジュール。サジェストは取得しないため
//! ネットワークには一切触れず、キー入力の経路で走っても表示レイテンシに
//! 影響しない。実際に開くのは `shell::open_shell_item` (既定ブラウザ)。

use serde::{Deserialize, Serialize};

/// 検索エンジン。設定で 1 つだけ選ぶ (FR-9.21)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    #[default]
    Google,
    Bing,
    DuckDuckGo,
    Brave,
}

impl Engine {
    /// 候補のパンくずに出す表示名。
    pub fn label(self) -> &'static str {
        match self {
            Engine::Google => "Google",
            Engine::Bing => "Bing",
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Brave => "Brave",
        }
    }

    /// 検索 URL のテンプレート。`{query}` を URL エンコード済みの語で置換する。
    fn template(self) -> &'static str {
        match self {
            Engine::Google => "https://www.google.com/search?q={query}",
            Engine::Bing => "https://www.bing.com/search?q={query}",
            Engine::DuckDuckGo => "https://duckduckgo.com/?q={query}",
            Engine::Brave => "https://search.brave.com/search?q={query}",
        }
    }

    /// 検索語が空のときに開くトップページ (FR-9.21)。
    fn home(self) -> &'static str {
        match self {
            Engine::Google => "https://www.google.com/",
            Engine::Bing => "https://www.bing.com/",
            Engine::DuckDuckGo => "https://duckduckgo.com/",
            Engine::Brave => "https://search.brave.com/",
        }
    }

    /// 検索語から実際に開く URL を組み立てる。語が空ならトップページ。
    pub fn url_for(self, query: &str) -> String {
        let query = query.trim();
        if query.is_empty() {
            return self.home().to_string();
        }
        self.template().replace("{query}", &encode(query))
    }
}

/// クエリ文字列向けの percent-encoding。
///
/// RFC 3986 の unreserved (`A-Z a-z 0-9 - . _ ~`) だけをそのまま通し、
/// 残りは UTF-8 のバイト単位で `%XX` にする。空白は `+` ではなく `%20`
/// にする (どのエンジンも受け付け、`+` 自体を検索したい場合と区別できる)。
fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_spaces_and_symbols() {
        assert_eq!(encode("rust lifetime"), "rust%20lifetime");
        assert_eq!(encode("a+b"), "a%2Bb");
        assert_eq!(encode("c#"), "c%23");
        assert_eq!(encode("foo-bar_baz.qux~1"), "foo-bar_baz.qux~1");
    }

    #[test]
    fn encodes_multibyte_per_utf8_byte() {
        // 「あ」= E3 81 82
        assert_eq!(encode("あ"), "%E3%81%82");
    }

    #[test]
    fn builds_search_url() {
        assert_eq!(
            Engine::Google.url_for("rust lifetime"),
            "https://www.google.com/search?q=rust%20lifetime"
        );
        assert_eq!(
            Engine::DuckDuckGo.url_for("rust"),
            "https://duckduckgo.com/?q=rust"
        );
    }

    #[test]
    fn empty_query_opens_home() {
        assert_eq!(Engine::Google.url_for(""), "https://www.google.com/");
        assert_eq!(Engine::Bing.url_for("   "), "https://www.bing.com/");
    }

    #[test]
    fn trims_surrounding_space() {
        assert_eq!(
            Engine::Google.url_for("  rust  "),
            "https://www.google.com/search?q=rust"
        );
    }
}
