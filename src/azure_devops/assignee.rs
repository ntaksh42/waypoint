//! Work Item の担当者が自分か (`az wit mine`) の判定。
//!
//! 共有キャッシュ (DevDeck と共有) の Work Item は担当者の表示名と uniqueName しか
//! 持たず、`current_user_id` で得る自分の ID と直接は比べられない。そこで同期で
//! 取得した Work Item のうち担当者 ID が自分と一致したものから自分の uniqueName を
//! 覚え (`learn_unique_name`)、以後は uniqueName で比べる。

use rusqlite::params;
use serde_json::Value;

use super::cache::{open_cache, open_cache_read_only};

/// 同期で取得した Work Item (`workitemsbatch` の応答) から、担当者 ID が `my_id` の
/// 項目を探し、その uniqueName を組織ごとに保存する。見つからなければ何もしない。
pub(crate) fn learn_unique_name(organization: &str, my_id: Option<&str>, items: &[Value]) {
    let Some(my_id) = my_id else {
        return;
    };
    let unique_name = items.iter().find_map(|item| {
        let assigned = &item["fields"]["System.AssignedTo"];
        (assigned["id"].as_str()?.eq_ignore_ascii_case(my_id))
            .then(|| assigned["uniqueName"].as_str())
            .flatten()
    });
    if let Some(unique_name) = unique_name
        && let Err(error) = write_unique_name(organization, unique_name)
    {
        crate::panic_log::record(&format!(
            "azure devops: could not cache unique name for {organization}: {error}"
        ));
    }
}

/// 覚えている自分の uniqueName。未学習なら `None`。
pub(crate) fn read_unique_name(organization: &str) -> Option<String> {
    let connection = open_cache_read_only().ok()?;
    connection
        .query_row(
            "SELECT unique_name FROM identity WHERE organization = ?1",
            params![organization.trim()],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|name| !name.is_empty())
}

fn write_unique_name(organization: &str, unique_name: &str) -> Result<(), String> {
    let connection = open_cache()?;
    // identity の行は `write_identity` が先に作る (同じ同期の冒頭で ID を保存する)
    connection
        .execute(
            "UPDATE identity SET unique_name = ?2 WHERE organization = ?1",
            params![organization.trim(), unique_name],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Work Item の担当者の uniqueName。Work Item Tracking API は
/// `System.AssignedTo` をオブジェクトで、Search API は `system.assignedto` を
/// `"表示名 <uniqueName>"` の文字列で返す。
pub(crate) fn assigned_unique_name(fields: &Value) -> Option<&str> {
    if let Some(name) = fields["System.AssignedTo"]["uniqueName"].as_str() {
        return Some(name);
    }
    let text = fields["system.assignedto"]
        .as_str()
        .or_else(|| fields["System.AssignedTo"].as_str())?;
    let start = text.rfind('<')?;
    text[start + 1..].strip_suffix('>')
}

/// 担当者が自分か。自分の uniqueName を知らなければ常に `false`。
pub(crate) fn is_me(assigned: Option<&str>, me: Option<&str>) -> bool {
    assigned
        .zip(me)
        .is_some_and(|(assigned, me)| assigned.trim().eq_ignore_ascii_case(me.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn assigned_unique_name_reads_both_api_shapes() {
        let batch = json!({ "System.AssignedTo": { "displayName": "Alice", "uniqueName": "alice@example.com" } });
        assert_eq!(assigned_unique_name(&batch), Some("alice@example.com"));
        let search = json!({ "system.assignedto": "Alice Smith <alice@example.com>" });
        assert_eq!(assigned_unique_name(&search), Some("alice@example.com"));
        assert_eq!(
            assigned_unique_name(&json!({ "system.assignedto": "Alice" })),
            None
        );
        assert_eq!(assigned_unique_name(&json!({})), None);
    }

    #[test]
    fn is_me_compares_case_insensitively_and_needs_both_names() {
        assert!(is_me(Some("Alice@Example.com"), Some("alice@example.com")));
        assert!(!is_me(Some("bob@example.com"), Some("alice@example.com")));
        assert!(!is_me(None, Some("alice@example.com")));
        assert!(!is_me(Some("alice@example.com"), None));
    }
}
