//! ブラウザの履歴・ブックマークで、同じページの URL 違いを畳むための重複判定キー。
//!
//! キーは判定専用で、開く URL には代表の元の文字列をそのまま使う。
//! 全件に掛かるので `reqwest::Url` へのパースはせず、文字列処理だけで済ませる。

/// 重複判定用のキーを返す。`http` / `https` 以外はそのまま返す。
///
/// - スキーム差 (`http` / `https`)、ホストの大小文字、パス末尾の `/` を無視する
/// - フラグメントは除去する。ただし SPA のルート (`#/` `#!`) は別ページなので保持する
/// - `utm_*` / `fbclid` / `gclid` などのトラッキングクエリを除去する
/// - パス・クエリの大小文字は区別する (別ページの履歴が消えるため)
pub fn duplicate_key(url: &str) -> String {
    let rest = match url.split_once("://").filter(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    }) {
        Some((_, rest)) => rest,
        None => return url.to_string(),
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (rest, None),
    };
    let (location, query) = match rest.split_once('?') {
        Some((location, query)) => (location, Some(query)),
        None => (rest, None),
    };
    let (host, path) = match location.find('/') {
        Some(index) => location.split_at(index),
        None => (location, ""),
    };
    let host = host.to_ascii_lowercase();
    let learn = host == "learn.microsoft.com";

    let mut key = host;
    key.push_str(path.trim_end_matches('/'));
    let params: Vec<&str> = query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter(|param| !param.is_empty() && !is_ignored_param(param, learn))
        .collect();
    if !params.is_empty() {
        key.push('?');
        key.push_str(&params.join("&"));
    }
    // Learn の `#tabs=` など記事内アンカーはページを識別しない
    if let Some(fragment) =
        fragment.filter(|f| !learn && (f.starts_with('/') || f.starts_with('!')))
    {
        key.push('#');
        key.push_str(fragment);
    }
    key
}

fn is_ignored_param(param: &str, learn: bool) -> bool {
    let name = param.split('=').next().unwrap_or(param);
    name.to_ascii_lowercase().starts_with("utm_")
        || matches!(
            name,
            "fbclid" | "gclid" | "dclid" | "msclkid" | "mc_eid" | "igshid"
        )
        || (learn && name == "tabs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_trailing_slash_fragment_and_tracking_are_ignored() {
        assert_eq!(
            duplicate_key("https://example.com/a"),
            duplicate_key("http://example.com/a/#top?utm_source=x")
        );
        assert_eq!(
            duplicate_key("https://Example.com/a?id=1&fbclid=z"),
            duplicate_key("https://example.com/a/?id=1")
        );
        assert_eq!(
            duplicate_key("https://example.com/"),
            duplicate_key("http://example.com")
        );
    }

    #[test]
    fn path_and_query_case_stay_distinct() {
        assert_ne!(
            duplicate_key("https://example.com/Foo"),
            duplicate_key("https://example.com/foo")
        );
        assert_ne!(
            duplicate_key("https://example.com/?Key=A"),
            duplicate_key("https://example.com/?Key=a")
        );
    }

    #[test]
    fn spa_routes_and_real_queries_are_kept() {
        assert_ne!(
            duplicate_key("https://example.com/#/one"),
            duplicate_key("https://example.com/#/two")
        );
        assert_ne!(
            duplicate_key("https://example.com/?tabs=one"),
            duplicate_key("https://example.com/?tabs=two")
        );
        assert_ne!(
            duplicate_key("https://example.com/?q=1"),
            duplicate_key("https://example.com/?q=2")
        );
    }

    #[test]
    fn learn_tabs_and_anchors_are_ignored_and_other_schemes_untouched() {
        assert_eq!(
            duplicate_key("https://learn.microsoft.com/a/b?view=net-8.0&tabs=csharp#examples"),
            duplicate_key("https://learn.microsoft.com/a/b?tabs=vb&view=net-8.0")
        );
        assert_eq!(duplicate_key("file:///C:/x/"), "file:///C:/x/");
    }
}
