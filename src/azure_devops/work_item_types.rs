//! Work Item Type ごとの色とアイコン (`GET _apis/wit/workitemtypes`)。
//!
//! Agile / Scrum / CMMI / Basic で Type が異なり、独自の Type も作れるため、
//! 色は Type 名からの推測ではなく API の値を使う。取得は同期時 (バックグラウンド)
//! に 1 回だけ行い、`azure_devops.db` に保存する。表示・入力の経路では
//! API も SQLite も叩かず、プロセス内のレジストリを引くだけにする。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use rusqlite::params;
use serde_json::Value;

use crate::config::AzureDevOpsProject;

use super::api::{API_VERSION, get_json};
use super::cache::{open_cache, open_cache_read_only};
use super::convert::encode_segment;

/// API が返す Type の見た目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemTypeStyle {
    pub color: (u8, u8, u8),
    /// `icon_insect` など。組込みの記号へ対応付けられなければ使わない。
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypeRow {
    name: String,
    style: WorkItemTypeStyle,
}

type Registry = HashMap<(String, String, String), WorkItemTypeStyle>;

fn registry() -> &'static RwLock<Registry> {
    static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
    // 初回だけ DB から読む。以後の更新は同期完了時の `store` が行う。
    REGISTRY.get_or_init(|| RwLock::new(load_all().unwrap_or_default()))
}

fn key(organization: &str, project: &str, name: &str) -> (String, String, String) {
    (
        organization.trim().to_lowercase(),
        project.trim().to_lowercase(),
        name.trim().to_lowercase(),
    )
}

/// 取得済みの Type の見た目。未取得・未知の Type は `None` (呼び出し側が従来の配色へ戻す)。
pub fn style_for(organization: &str, project: &str, name: &str) -> Option<WorkItemTypeStyle> {
    registry()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key(organization, project, name))
        .cloned()
}

/// `color` は `CC293D` のような 16 進 6 桁 (`#` 付きや前後の空白は許す)。
fn parse_color(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// 色を持たない Type は捨てる (見た目を決められないので従来の配色に任せる)。
fn parse_types(value: &Value) -> Vec<TypeRow> {
    value["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item["name"].as_str()?.to_string();
            let color = parse_color(item["color"].as_str()?)?;
            let icon = item["icon"]["id"].as_str().map(str::to_string);
            Some(TypeRow {
                name,
                style: WorkItemTypeStyle { color, icon },
            })
        })
        .collect()
}

/// 同期の一部として呼ぶ。API → DB → レジストリの順に更新する。
pub(crate) fn sync_work_item_types(
    client: &reqwest::blocking::Client,
    project: &AzureDevOpsProject,
    pat: &str,
) -> Result<(), String> {
    let url = format!(
        "https://dev.azure.com/{}/{}/_apis/wit/workitemtypes?api-version={API_VERSION}",
        encode_segment(&project.organization),
        encode_segment(&project.project)
    );
    let rows = parse_types(&get_json(client, &url, pat)?);
    replace_in_db(project, &rows)?;
    let mut registry = registry()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (organization, project_name) = (
        project.organization.trim().to_lowercase(),
        project.project.trim().to_lowercase(),
    );
    registry.retain(|(org, proj, _), _| *org != organization || *proj != project_name);
    for row in rows {
        registry.insert(key(&organization, &project_name, &row.name), row.style);
    }
    Ok(())
}

fn replace_in_db(project: &AzureDevOpsProject, rows: &[TypeRow]) -> Result<(), String> {
    let mut connection = open_cache()?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let (organization, project_name) = (project.organization.trim(), project.project.trim());
    transaction
        .execute(
            "DELETE FROM work_item_types WHERE organization = ?1 AND project = ?2",
            params![organization, project_name],
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (red, green, blue) = row.style.color;
        transaction
            .execute(
                "INSERT OR REPLACE INTO work_item_types
                 (organization, project, name, color, icon) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    organization,
                    project_name,
                    row.name,
                    format!("{red:02X}{green:02X}{blue:02X}"),
                    row.style.icon
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// DB が無い・表が無い (まだ同期していない) 場合は `Err` を返し、呼び出し側が空にする。
fn load_all() -> Result<Registry, String> {
    let connection = open_cache_read_only()?;
    let mut statement = connection
        .prepare("SELECT organization, project, name, color, icon FROM work_item_types")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|(organization, project, name, color, icon)| {
            Some((
                key(&organization, &project, &name),
                WorkItemTypeStyle {
                    color: parse_color(&color)?,
                    icon,
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colors_parse_from_six_hex_digits() {
        assert_eq!(parse_color("CC293D"), Some((0xCC, 0x29, 0x3D)));
        assert_eq!(parse_color(" #009ccc "), Some((0x00, 0x9C, 0xCC)));
        assert_eq!(parse_color("FFF"), None);
        assert_eq!(parse_color("GG0000"), None);
        assert_eq!(parse_color("あいう"), None);
    }

    #[test]
    fn api_response_keeps_types_with_a_color_and_drops_the_rest() {
        let value = json!({ "value": [
            { "name": "Bug", "color": "CC293D", "icon": { "id": "icon_insect" } },
            { "name": "Product Backlog Item", "color": "009CCC" },
            { "name": "No Color", "icon": { "id": "icon_book" } },
            { "color": "FFFFFF" },
        ]});
        let rows = parse_types(&value);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Bug");
        assert_eq!(rows[0].style.icon.as_deref(), Some("icon_insect"));
        assert_eq!(rows[1].style.color, (0x00, 0x9C, 0xCC));
        assert_eq!(rows[1].style.icon, None);
        assert!(parse_types(&json!({})).is_empty());
    }

    #[test]
    fn lookup_ignores_case_and_surrounding_spaces() {
        registry().write().unwrap().insert(
            key("Org-Lookup", "Proj", "User Story"),
            WorkItemTypeStyle {
                color: (1, 2, 3),
                icon: None,
            },
        );
        let found = style_for(" org-lookup ", "PROJ", "user story").unwrap();
        assert_eq!(found.color, (1, 2, 3));
        assert!(style_for("org-lookup", "proj", "Bug").is_none());
    }
}
