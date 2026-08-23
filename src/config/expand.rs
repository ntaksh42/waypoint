//! パス中の変数展開 (FR-5.1 / FR-5.2) 。

use std::collections::BTreeMap;

/// パス中の `%ENV%` と `{UserVar}` を展開する。
///
/// 解決できない変数が残った場合は None を返し、呼び出し側で
/// グレー表示にする (FR-5.4) 。
pub fn expand(path: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let expanded = expand_user_vars(path, vars)?;
    expand_env_vars(&expanded)
}

fn expand_user_vars(input: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('{') {
        let end = rest[start..].find('}')? + start;
        out.push_str(&rest[..start]);
        let key = &rest[start + 1..end];
        out.push_str(vars.get(key)?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn expand_env_vars(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        let end = rest[start + 1..].find('%')? + start + 1;
        out.push_str(&rest[..start]);
        let key = &rest[start + 1..end];
        // %% はリテラルの % として扱う
        if key.is_empty() {
            out.push('%');
        } else {
            out.push_str(&std::env::var(key).ok()?);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}
