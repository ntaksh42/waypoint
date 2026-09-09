//! 「Suggest priorities from recent activity」用の直近アクティビティ集計。

use std::collections::HashMap;
use std::thread;

use crate::config::AzureDevOpsSettings;

use super::super::api::{RecentActivityPaths, fetch_recent_activity_paths, http_client};
use super::super::auth_cache::OrganizationValues;
use super::super::convert::valid_project;
use super::super::credential::load_pat;
use super::common::join_worker;

/// 監視プロジェクト 1 件分の、直近アクティビティ件数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectActivity {
    pub organization: String,
    pub project: String,
    pub count: usize,
    /// このプロジェクト内での Area Path 別件数 (件数の多い順)。
    pub areas: Vec<(String, usize)>,
    /// このプロジェクト内での Iteration Path 別件数 (件数の多い順)。
    pub iterations: Vec<(String, usize)>,
}

/// 設定画面の「Suggest priorities from recent activity」用。監視中の全
/// プロジェクトを並列に叩き、直近 90 日の自分の作業件数を集計する。
/// 一部の API が失敗したプロジェクトは 0 件として結果に含める。全件失敗は
/// 呼び出し側へエラーを返す。設定画面は
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
                Result<RecentActivityPaths, String>,
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
                                Some(Ok(pat)) => fetch_recent_activity_paths(client, project, pat),
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
                    .zip(targets.iter().copied())
                    .map(|(handle, project)| {
                        join_worker(handle).unwrap_or_else(|error| (project, Err(error)))
                    })
                    .collect()
            });
            let failures: Vec<_> = outcomes
                .iter()
                .filter_map(|(project, outcome)| {
                    outcome.as_ref().err().map(|error| {
                        format!("{}/{}: {error}", project.organization, project.project)
                    })
                })
                .collect();
            if !outcomes.is_empty() && failures.len() == outcomes.len() {
                return Err(format!(
                    "Could not load recent activity: {}",
                    failures.join("; ")
                ));
            }
            Ok(outcomes
                .into_iter()
                .map(|(project, outcome)| match outcome {
                    Ok(paths) => ProjectActivity {
                        organization: project.organization.trim().to_string(),
                        project: project.project.trim().to_string(),
                        count: paths.iterations.len(),
                        areas: count_paths(paths.areas),
                        iterations: count_paths(paths.iterations),
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
                            iterations: Vec::new(),
                        }
                    }
                })
                .collect())
        })();
        let _ = sender.send(result);
    });
    receiver
}

/// 直近アクティビティの多い順に Project の優先度を振り直し、各 Project の
/// 最頻 Iteration Path を Work Item 検索の対象へ設定する。アクティビティが
/// 無い Project は既存の優先度順で末尾へ置く。
pub fn apply_optimization(
    settings: &mut AzureDevOpsSettings,
    activity: &[ProjectActivity],
) -> usize {
    let mut activity_by_project = HashMap::new();
    for entry in activity {
        activity_by_project.insert(
            (
                entry.organization.to_lowercase(),
                entry.project.to_lowercase(),
            ),
            (
                entry.count,
                entry.iterations.first().map(|(path, _)| path.clone()),
            ),
        );
    }
    let mut ranked: Vec<_> = settings
        .projects
        .iter()
        .enumerate()
        .map(|(index, project)| {
            let key = (
                project.organization.to_lowercase(),
                project.project.to_lowercase(),
            );
            let (count, iteration) = activity_by_project.get(&key).cloned().unwrap_or((0, None));
            (index, count, iteration, project.priority)
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.3.cmp(&right.3))
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut optimized = 0;
    for (priority, (index, count, iteration, _)) in ranked.into_iter().enumerate() {
        let project = &mut settings.projects[index];
        project.priority = priority as u32;
        if count > 0 {
            project.interest_iterations = iteration.into_iter().collect();
            optimized += 1;
        }
    }
    optimized
}

/// Area Path の出現回数を多い順に集計する。
fn count_paths(paths: Vec<String>) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for path in paths {
        *counts.entry(path).or_insert(0) += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_paths_orders_by_frequency_then_name() {
        let counts = count_paths(vec![
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
    fn count_paths_is_empty_without_input() {
        assert!(count_paths(Vec::new()).is_empty());
    }

    #[test]
    fn optimization_ranks_recent_projects_and_uses_their_top_iteration() {
        let mut settings = AzureDevOpsSettings {
            enabled: true,
            projects: vec![
                project("contoso", "Idle", 0),
                project("contoso", "Active", 1),
            ],
        };
        let activity = vec![ProjectActivity {
            organization: "contoso".to_string(),
            project: "Active".to_string(),
            count: 3,
            areas: Vec::new(),
            iterations: vec![("Active\\Sprint 2".to_string(), 3)],
        }];

        assert_eq!(apply_optimization(&mut settings, &activity), 1);
        assert_eq!(settings.projects[1].priority, 0);
        assert_eq!(settings.projects[0].priority, 1);
        assert_eq!(
            settings.projects[1].interest_iterations,
            ["Active\\Sprint 2"]
        );
        assert!(settings.projects[0].interest_iterations.is_empty());
    }

    fn project(
        organization: &str,
        project: &str,
        priority: u32,
    ) -> crate::config::AzureDevOpsProject {
        crate::config::AzureDevOpsProject {
            organization: organization.to_string(),
            project: project.to_string(),
            aliases: Vec::new(),
            priority,
            include_pull_requests: true,
            include_pipelines: true,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: Vec::new(),
        }
    }
}
