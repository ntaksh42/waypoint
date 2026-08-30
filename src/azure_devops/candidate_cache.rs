#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure_devops::{
        CachedCandidateGroups, Candidate, Kind, try_cached_candidate_groups,
    };
    use crate::config::AzureDevOpsSettings;

    fn candidate(kind: Kind, name: &str) -> Candidate {
        Candidate {
            kind,
            status: String::new(),
            name: name.into(),
            detail: "Azure DevOps — org/project".into(),
            url: "https://dev.azure.com/org/project".into(),
            organization: "org".into(),
            project: "project".into(),
            aliases: Vec::new(),
            priority: 0,
            is_mine: false,
        }
    }

    #[test]
    fn candidate_group_result_keeps_both_groups_when_cache_is_healthy() {
        let pull_requests = vec![candidate(Kind::PullRequest, "PR 1")];
        let work_items = vec![candidate(Kind::WorkItem, "42: Fix")];
        let groups =
            finish_candidate_groups(Ok((pull_requests.clone(), work_items.clone())), vec![])
                .unwrap();
        assert_eq!(groups.pull_requests, pull_requests);
        assert_eq!(groups.work_items, work_items);
    }

    #[test]
    fn candidate_group_result_reports_shared_cache_failure() {
        let result = finish_candidate_groups(Err("shared cache unavailable".into()), vec![]);
        assert_eq!(result.unwrap_err(), "shared cache unavailable");
    }

    #[test]
    fn checked_candidate_groups_are_reexported_by_azure_devops() {
        let _: fn(&AzureDevOpsSettings) -> Result<CachedCandidateGroups, String> =
            try_cached_candidate_groups;
    }
}
use std::collections::HashMap;

use crate::config::AzureDevOpsSettings;

use super::cache::{CachedRow, open_cache_read_only, read_identity};
use super::convert::{project_key, valid_project};
use super::{Candidate, Kind, shared_cache};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CachedCandidateGroups {
    pub(crate) pull_requests: Vec<Candidate>,
    pub(crate) work_items: Vec<Candidate>,
}

/// Active PR 候補。共有キャッシュ (`shared_cache`) を読む。フレッシュで
/// なければ waypoint 自身がバックグラウンドで更新する
/// (`api.rs::refresh_project`) ので、ここは常に「今ある内容」を返すだけ。
pub fn cached_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    cached_candidate_groups(settings).0
}

/// waypoint 自身が保持する PR 履歴 (Completed/Abandoned、過去 90 日ぶん)。
/// DevDeck は Active PR しか同期しないので、この分だけは重複しておらず
/// waypoint 側のキャッシュのまま残す (`api.rs::fetch_pull_requests` 参照)。
/// 読めなければ空で続行する。読み取り専用接続を使い、waypoint 自身の
/// バックグラウンド同期の書き込みと競合しない。
fn pr_history_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    let Ok(connection) = open_cache_read_only() else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT organization, project, kind, item_id, status, name, detail, url, is_mine
         FROM candidates WHERE kind = 'pr'",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok(CachedRow {
            organization: row.get(0)?,
            project: row.get(1)?,
            kind: row.get(2)?,
            item_id: row.get(3)?,
            status: row.get(4)?,
            name: row.get(5)?,
            detail: row.get(6)?,
            url: row.get(7)?,
            is_mine: row.get::<_, i64>(8)? != 0,
        })
    }) else {
        return Vec::new();
    };

    let configured: HashMap<_, _> = settings
        .projects
        .iter()
        .filter(|project| valid_project(project) && project.include_pull_requests)
        .map(|project| {
            (
                project_key(&project.organization, &project.project),
                (project.aliases.clone(), project.priority),
            )
        })
        .collect();
    rows.filter_map(Result::ok)
        .filter_map(|row| {
            let (aliases, priority) = configured
                .get(&project_key(&row.organization, &row.project))?
                .clone();
            Some(Candidate {
                kind: Kind::PullRequest,
                status: row.status,
                name: row.name,
                detail: row.detail,
                url: row.url,
                organization: row.organization,
                project: row.project,
                aliases,
                priority,
                is_mine: row.is_mine,
            })
        })
        .collect()
}

/// Work Item 候補。共有キャッシュ (`shared_cache`) を読む
/// (`az wit live` はライブ検索で API を直接叩くので対象外)。
/// 読めなければ空で続行する。
pub fn cached_work_item_candidates(settings: &AzureDevOpsSettings) -> Vec<Candidate> {
    cached_candidate_groups(settings).1
}

/// Active PR と Work Item を共有キャッシュ (`shared_cache`) の同じ接続から
/// まとめて読む (接続を開き直すコストを 1 回で済ませる)。失敗時は互換 API
/// として従来どおり waypoint 自身の PR 履歴だけへフォールバックする。
pub(crate) fn cached_candidate_groups(
    settings: &AzureDevOpsSettings,
) -> (Vec<Candidate>, Vec<Candidate>) {
    match try_cached_candidate_groups(settings) {
        Ok(groups) => (groups.pull_requests, groups.work_items),
        Err(_) => (pr_history_candidates(settings), Vec::new()),
    }
}

/// 共有キャッシュの候補群を読み、クエリ失敗を呼び出し元へ明示する。
pub(crate) fn try_cached_candidate_groups(
    settings: &AzureDevOpsSettings,
) -> Result<CachedCandidateGroups, String> {
    if !settings.enabled {
        return Ok(CachedCandidateGroups::default());
    }
    let shared = shared_cache::with_cached_connection(|connection| {
        let (groups, healthy) = read_candidate_groups(connection, settings);
        (
            healthy
                .then_some(groups)
                .ok_or_else(|| "Azure shared cache query failed.".to_string()),
            healthy,
        )
    })??;
    finish_candidate_groups(Ok(shared), pr_history_candidates(settings))
}

fn finish_candidate_groups(
    shared: Result<(Vec<Candidate>, Vec<Candidate>), String>,
    history: Vec<Candidate>,
) -> Result<CachedCandidateGroups, String> {
    let (mut pull_requests, work_items) = shared?;
    pull_requests.extend(history);
    Ok(CachedCandidateGroups {
        pull_requests,
        work_items,
    })
}

/// 共有キャッシュ読取の本体。`healthy` は呼び出し元のキャッシュ接続を
/// 使い回してよいかの判定に使う — スキーマ不一致等でクエリ自体が失敗した
/// 場合だけ `false` にし、次回呼び出しで接続を開き直させる。
fn read_candidate_groups(
    connection: &rusqlite::Connection,
    settings: &AzureDevOpsSettings,
) -> ((Vec<Candidate>, Vec<Candidate>), bool) {
    let mut pull_requests = Vec::new();
    let mut work_items = Vec::new();
    let mut healthy = true;
    for project in &settings.projects {
        if !valid_project(project) {
            continue;
        }
        let organization = project.organization.trim();
        let project_name = project.project.trim();

        if project.include_pull_requests {
            match shared_cache::read_pull_requests(connection, organization, project_name) {
                Ok(rows) => {
                    let reviewers =
                        shared_cache::read_reviewers(connection, organization, project_name)
                            .unwrap_or_default();
                    let my_id = read_identity(organization);
                    for row in rows {
                        let Some(url) = row.web_url else { continue };
                        let is_mine = my_id.as_deref().is_some_and(|my_id| {
                            row.created_by_id.as_deref() == Some(my_id)
                                || reviewers.iter().any(|reviewer| {
                                    reviewer.repository_id == row.repository_id
                                        && reviewer.pull_request_id == row.pull_request_id
                                        && reviewer.reviewer_id == my_id
                                })
                        });
                        pull_requests.push(Candidate {
                            kind: Kind::PullRequest,
                            name: format!("PR {}: {}", row.pull_request_id, row.title),
                            detail: match &row.created_by {
                                Some(author) if !author.is_empty() => format!(
                                    "Azure DevOps — {organization}/{project_name} — {} — by {author}",
                                    row.status
                                ),
                                _ => format!(
                                    "Azure DevOps — {organization}/{project_name} — {}",
                                    row.status
                                ),
                            },
                            status: row.status,
                            url,
                            organization: organization.to_string(),
                            project: project_name.to_string(),
                            aliases: project.aliases.clone(),
                            priority: project.priority,
                            is_mine,
                        });
                    }
                }
                Err(_) => healthy = false,
            }
        }

        if project.include_work_items {
            match shared_cache::read_work_items(connection, organization, project_name) {
                Ok(rows) => {
                    for row in rows {
                        let Some(url) = row.web_url else { continue };
                        let kind = row.work_item_type.as_deref().unwrap_or("Work Item");
                        let state = row.state.as_deref().unwrap_or("");
                        work_items.push(Candidate {
                            kind: Kind::WorkItem,
                            status: state.to_string(),
                            name: format!("{}: {}", row.id, row.title),
                            detail: format!(
                                "Azure DevOps — {organization}/{project_name} — {kind} {state}"
                            ),
                            url,
                            organization: organization.to_string(),
                            project: project_name.to_string(),
                            aliases: project.aliases.clone(),
                            priority: project.priority,
                            is_mine: false,
                        });
                    }
                }
                Err(_) => healthy = false,
            }
        }
    }
    ((pull_requests, work_items), healthy)
}
