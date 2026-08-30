//! `az pr` のライブ検索。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::AzureDevOpsSettings;

use super::super::Candidate;
use super::super::api::{current_user_id, fetch_pull_requests_live, http_client};
use super::super::auth_cache::OrganizationValues;
use super::super::convert::{pull_request_cached_row_to_candidate, valid_project};
use super::super::credential::load_pat;
use super::common::lock_recovering;
use super::work_items::WorkItemReply;

/// PR のライブ検索結果。フィールドは `WorkItemReply` と同じ形だが、
/// `pending_work_items` と混ざらないよう独立した `reply_id` 空間を使う。
pub type PullRequestReply = WorkItemReply;

/// プロジェクト × ステータス 1 組の PR ライブ検索の結果。
type PullRequestFetchOutcome<'a> = (
    &'a crate::config::AzureDevOpsProject,
    &'static str,
    Result<Vec<Candidate>, String>,
);

/// `az pr` 等がキャッシュ検索で 0 件だったとき、ユーザーが明示的に選んで
/// 叫ぶライブ検索。打ち切り期間を広げて対象ステータスを再取得し、
/// ローカルで `mine` / 検索語をフィルタする。監視プロジェクトが複数でも
/// `az wit` のライブ検索と同様に並列で投げる (プロジェクト × ステータスの
/// 組ごとに 1 リクエスト、`PullRequestStatus::All` なら completed と
/// abandoned の両方を同時に叩く)。
pub fn search_pull_requests_live_async(
    settings: AzureDevOpsSettings,
    statuses: &'static [&'static str],
    mine: bool,
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
                    .filter(|project| valid_project(project) && project.include_pull_requests)
                    .collect();
                let auth = OrganizationValues::new(
                    targets.iter().map(|project| project.organization.as_str()),
                );
                let jobs: Vec<(&crate::config::AzureDevOpsProject, &'static str)> = targets
                    .iter()
                    .flat_map(|project| statuses.iter().map(move |status| (*project, *status)))
                    .collect();
                let outcomes: Vec<PullRequestFetchOutcome> = thread::scope(|scope| {
                    let handles: Vec<_> = jobs
                        .iter()
                        .map(|&(project, status)| {
                            let client = &client;
                            let auth = &auth;
                            scope.spawn(move || {
                                let outcome = match auth.get_or_init(&project.organization, || {
                                    load_pat(&project.organization).map(|pat| {
                                        let user =
                                            current_user_id(client, &project.organization, &pat)
                                                .ok();
                                        (pat, user)
                                    })
                                }) {
                                    Some(Ok((pat, user))) => fetch_pull_requests_live(
                                        client,
                                        project,
                                        pat,
                                        user.as_deref(),
                                        status,
                                    )
                                    .map(|rows| {
                                        rows.iter()
                                            .map(|row| {
                                                pull_request_cached_row_to_candidate(project, row)
                                            })
                                            .collect()
                                    }),
                                    Some(Err(_)) | None => {
                                        Err(format!("{}: no PAT", project.organization))
                                    }
                                };
                                (project, status, outcome)
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| handle.join().expect("pull request fetch thread panicked"))
                        .collect()
                });
                for (project, status, outcome) in outcomes {
                    match outcome {
                        Ok(found) => results.extend(found),
                        Err(error) => {
                            crate::panic_log::record(&format!(
                                "azure devops: live pull request search {}/{} ({status}) failed: {error}",
                                project.organization, project.project
                            ));
                            failures.push(format!("{}/{}", project.organization, project.project));
                        }
                    }
                }
            }
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: could not initialize pull request client: {error}"
            )),
        }
        let terms = query.trim().to_lowercase();
        if !terms.is_empty() {
            results.retain(|candidate| candidate.name.to_lowercase().contains(&terms));
        }
        if mine {
            results.retain(|candidate| candidate.is_mine);
        }
        results.sort_by_key(|candidate| (candidate.priority, candidate.name.to_lowercase()));
        failures.sort();
        failures.dedup();
        let empty_message = if results.is_empty() {
            if failures.is_empty() {
                Some("No matching pull requests.".to_string())
            } else {
                Some(format!(
                    "Azure DevOps search unavailable ({})",
                    failures.join(", ")
                ))
            }
        } else {
            None
        };
        let mut pending = lock_recovering(pending_pull_requests());
        pending.insert(
            request_id,
            PullRequestReply {
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

pub fn take_pull_request_results(request_id: u32) -> Option<PullRequestReply> {
    lock_recovering(pending_pull_requests()).remove(&request_id)
}

fn pending_pull_requests() -> &'static Mutex<HashMap<u32, PullRequestReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, PullRequestReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}
