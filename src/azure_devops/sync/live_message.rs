//! Live 結果の範囲・部分失敗を、0件のときだけに限定せず伝える。

pub(super) fn live_message(
    empty: bool,
    failures: &[String],
    no_results: &str,
    scope: &str,
) -> String {
    let mut parts = Vec::new();
    if !failures.is_empty() {
        parts.push(format!(
            "{} ({})",
            if empty {
                "Search unavailable"
            } else {
                "Partial results"
            },
            failures.join(", ")
        ));
    } else if empty {
        parts.push(no_results.to_string());
    }
    parts.push(scope.to_string());
    parts.join(" — ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_is_visible_even_when_other_projects_return_results() {
        let message = live_message(
            false,
            &["org/failed".into()],
            "No matches",
            "Live PR: Active",
        );
        assert!(message.contains("Partial results (org/failed)"));
        assert!(message.contains("Active"));
        assert!(!message.contains("No matches"));
        assert!(live_message(true, &[], "No matches", "First 100 builds").contains("No matches"));
        assert!(
            live_message(true, &["org/failed".into()], "No matches", "Live")
                .contains("Search unavailable")
        );
    }
}
