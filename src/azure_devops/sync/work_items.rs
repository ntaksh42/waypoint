//! `az wit ` のライブ検索。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::AzureDevOpsSettings;

use super::super::Candidate;
use super::super::api::{fetch_work_items, http_client};
use super::super::auth_cache::OrganizationValues;
use super::super::convert::valid_project;
use super::super::credential::load_pat;
use super::common::lock_recovering;

#[derive(Debug, Clone, Default)]
pub struct WorkItemReply {
    pub candidates: Vec<Candidate>,
    pub message: Option<String>,
}

/// `az wit ` の検索をバックグラウンドで実行する。結果は ID ごとに保持し、
/// 呼び出し側が最新 ID と一致したものだけを表示する。
///
/// 監視プロジェクトが複数ある場合、プロジェクトごとに順番に HTTP 応答を
/// 待つと合計待ち時間が積み上がり体感で遅くなる (実測で報告あり)。
/// プロジェクト間は互いに独立な読み取りなので `thread::scope` で並列に
/// 投げ、全部揃うのを待ってからまとめる。
pub fn search_work_items_async(
    settings: AzureDevOpsSettings,
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
                    .filter(|project| valid_project(project) && project.include_work_items)
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
                            let query = &query;
                            scope.spawn(move || {
                                let outcome = match pats.get_or_init(&project.organization, || {
                                    load_pat(&project.organization)
                                }) {
                                    Some(Ok(pat)) => fetch_work_items(client, project, pat, query),
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
                        .map(|handle| handle.join().expect("work item fetch thread panicked"))
                        .collect()
                });
                for (project, outcome) in outcomes {
                    match outcome {
                        Ok(mut found) => results.append(&mut found),
                        Err(error) => {
                            crate::panic_log::record(&format!(
                                "azure devops: work item search {}/{} failed: {error}",
                                project.organization, project.project
                            ));
                            failures.push(format!("{}/{}", project.organization, project.project));
                        }
                    }
                }
            }
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: could not initialize work item client: {error}"
            )),
        }
        // `az wit` 単体は WIQL の更新日時順を保つ。検索語がある場合だけ
        // 名前順へそろえ、プロジェクト優先度はどちらにも適用する。
        if query.trim().is_empty() {
            results.sort_by_key(|candidate| candidate.priority);
        } else {
            results.sort_by_key(|candidate| (candidate.priority, candidate.name.to_lowercase()));
        }
        let empty_message = if results.is_empty() {
            if failures.is_empty() {
                Some(if query.trim().is_empty() {
                    "No recently updated work items.".to_string()
                } else {
                    "No matching work items.".to_string()
                })
            } else {
                Some(format!(
                    "Azure DevOps search unavailable ({})",
                    failures.join(", ")
                ))
            }
        } else {
            None
        };
        let mut pending = lock_recovering(pending_work_items());
        pending.insert(
            request_id,
            WorkItemReply {
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

pub fn take_work_item_results(request_id: u32) -> Option<WorkItemReply> {
    lock_recovering(pending_work_items()).remove(&request_id)
}

fn pending_work_items() -> &'static Mutex<HashMap<u32, WorkItemReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, WorkItemReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}
