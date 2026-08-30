//! `az pipeline ` のライブ検索。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::AzureDevOpsSettings;

use super::super::Candidate;
use super::super::api::{fetch_pipelines, http_client};
use super::super::auth_cache::OrganizationValues;
use super::super::convert::{pipeline_cached_row_to_candidate, valid_project};
use super::super::credential::load_pat;
use super::common::lock_recovering;
use super::work_items::WorkItemReply;

/// Pipeline のライブ検索結果。同じ理由で独立した `reply_id` 空間を使う。
pub type PipelineReply = WorkItemReply;

/// `az pipeline ` の検索をバックグラウンドで実行する。Pipeline は永続
/// キャッシュを持たないので (`az pr` / `az wit` と違い) 常にこの経路を通り、
/// 監視プロジェクトごとに並列で叩いてからローカルでステータス・検索語を
/// フィルタする。
pub fn search_pipelines_live_async(
    settings: AzureDevOpsSettings,
    filter: super::super::PipelineFilter,
    query: String,
    request_id: u32,
    notify: HWND,
    message: u32,
) {
    let notify = notify.0 as isize;
    thread::spawn(move || {
        let mut results = Vec::new();
        let mut failures = Vec::new();
        match http_client() {
            Ok(client) => {
                let targets: Vec<_> = settings
                    .projects
                    .iter()
                    .filter(|project| valid_project(project) && project.include_pipelines)
                    .collect();
                let pats = OrganizationValues::new(
                    targets.iter().map(|project| project.organization.as_str()),
                );
                let outcomes: Vec<(
                    &crate::config::AzureDevOpsProject,
                    Result<Vec<Candidate>, String>,
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
                                    Some(Ok(pat)) => {
                                        fetch_pipelines(client, project, pat).map(|rows| {
                                            rows.iter()
                                                .map(|row| {
                                                    pipeline_cached_row_to_candidate(project, row)
                                                })
                                                .collect()
                                        })
                                    }
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
                        .map(|handle| handle.join().expect("pipeline fetch thread panicked"))
                        .collect()
                });
                for (project, outcome) in outcomes {
                    match outcome {
                        Ok(found) => results.extend(found),
                        Err(error) => {
                            crate::panic_log::record(&format!(
                                "azure devops: live pipeline search {}/{} failed: {error}",
                                project.organization, project.project
                            ));
                            failures.push(format!("{}/{}", project.organization, project.project));
                        }
                    }
                }
            }
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: could not initialize pipeline client: {error}"
            )),
        }
        results.retain(|candidate: &Candidate| filter.matches(&candidate.status));
        let terms = query.trim().to_lowercase();
        if !terms.is_empty() {
            results.retain(|candidate| candidate.name.to_lowercase().contains(&terms));
        }
        results.sort_by_key(|candidate| (candidate.priority, candidate.name.to_lowercase()));
        failures.sort();
        failures.dedup();
        let empty_message = if results.is_empty() {
            if failures.is_empty() {
                Some("No matching pipelines.".to_string())
            } else {
                Some(format!(
                    "Azure DevOps search unavailable ({})",
                    failures.join(", ")
                ))
            }
        } else {
            None
        };
        let mut pending = lock_recovering(pending_pipelines());
        pending.insert(
            request_id,
            PipelineReply {
                candidates: results,
                message: empty_message,
            },
        );
        pending.retain(|id, _| *id >= request_id.saturating_sub(3));
        drop(pending);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(notify as *mut _)),
                message,
                WPARAM(request_id as usize),
                LPARAM(0),
            );
        }
    });
}

pub fn take_pipeline_results(request_id: u32) -> Option<PipelineReply> {
    lock_recovering(pending_pipelines()).remove(&request_id)
}

fn pending_pipelines() -> &'static Mutex<HashMap<u32, PipelineReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, PipelineReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}
