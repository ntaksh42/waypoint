//! 「Suggest priorities from recent activity」用の直近アクティビティ集計。

use std::collections::HashMap;
use std::thread;

use crate::config::AzureDevOpsSettings;

use super::super::api::{fetch_recent_activity_areas, http_client};
use super::super::auth_cache::OrganizationValues;
use super::super::convert::valid_project;
use super::super::credential::load_pat;

/// 監視プロジェクト 1 件分の、直近アクティビティ (アサイン + メンション) 件数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectActivity {
    pub organization: String,
    pub project: String,
    pub count: usize,
    /// このプロジェクト内での Area Path 別件数 (件数の多い順)。
    pub areas: Vec<(String, usize)>,
}

/// 設定画面の「Suggest priorities from recent activity」用。監視中の全
/// プロジェクトを並列に叩き、直近 90 日のアサイン・メンション件数を集計する。
/// API が失敗したプロジェクトは 0 件として結果に含める (取得できなかった
/// ことが UI から分かるよう、失敗はログにだけ残す)。設定画面は
/// `start_area_load` 等と同じく `Receiver` を直接ポーリングする方式なので、
/// ここも HWND 通知は使わない。
pub fn suggest_priorities_async(
    settings: AzureDevOpsSettings,
) -> std::sync::mpsc::Receiver<Result<Vec<ProjectActivity>, String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<Vec<ProjectActivity>, String> {
            let client = http_client()?;
            let targets: Vec<_> = settings
                .projects
                .iter()
                .filter(|project| valid_project(project))
                .collect();
            let pats = OrganizationValues::new(
                targets.iter().map(|project| project.organization.as_str()),
            );
            let outcomes: Vec<(
                &crate::config::AzureDevOpsProject,
                Result<Vec<String>, String>,
            )> = thread::scope(|scope| {
                let handles: Vec<_> = targets
                    .iter()
                    .map(|project| {
                        let client = &client;
                        let pats = &pats;
                        scope.spawn(move || {
                            let outcome = match pats.get_or_init(&project.organization, || {
                                load_pat(&project.organization)
                            }) {
                                Some(Ok(pat)) => fetch_recent_activity_areas(client, project, pat),
                                Some(Err(_)) | None => {
                                    Err(format!("{}: no PAT", project.organization))
                                }
                            };
                            (*project, outcome)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("activity fetch thread panicked"))
                    .collect()
            });
            Ok(outcomes
                .into_iter()
                .map(|(project, outcome)| match outcome {
                    Ok(areas) => ProjectActivity {
                        organization: project.organization.trim().to_string(),
                        project: project.project.trim().to_string(),
                        count: areas.len(),
                        areas: count_areas(areas),
                    },
                    Err(error) => {
                        crate::panic_log::record(&format!(
                            "azure devops: recent activity {}/{} failed: {error}",
                            project.organization, project.project
                        ));
                        ProjectActivity {
                            organization: project.organization.trim().to_string(),
                            project: project.project.trim().to_string(),
                            count: 0,
                            areas: Vec::new(),
                        }
                    }
                })
                .collect())
        })();
        let _ = sender.send(result);
    });
    receiver
}

/// Area Path の出現回数を多い順に集計する。
fn count_areas(areas: Vec<String>) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for area in areas {
        *counts.entry(area).or_insert(0) += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_areas_orders_by_frequency_then_name() {
        let counts = count_areas(vec![
            "Waypoint\\Search".to_string(),
            "Waypoint\\Launcher".to_string(),
            "Waypoint\\Launcher".to_string(),
        ]);
        assert_eq!(
            counts,
            vec![
                ("Waypoint\\Launcher".to_string(), 2),
                ("Waypoint\\Search".to_string(), 1),
            ]
        );
    }

    #[test]
    fn count_areas_is_empty_without_input() {
        assert!(count_areas(Vec::new()).is_empty());
    }
}
