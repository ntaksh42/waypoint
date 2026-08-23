//! バックグラウンド同期: プロジェクト全体のキャッシュ更新と、
//! `az wit ` のライブ Work Item 検索。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::AzureDevOpsSettings;

use super::Candidate;
use super::api::{fetch_work_items, http_client, refresh_project};
use super::cache;
use super::convert::valid_project;
use super::credential::load_pat;

#[derive(Debug, Clone, Default)]
pub struct WorkItemReply {
    pub candidates: Vec<Candidate>,
    pub message: Option<String>,
}

/// 同期を一つに直列化する。設定保存と手動更新が重なっても API と DB を競合させない。
pub(crate) static REFRESHING: AtomicBool = AtomicBool::new(false);

/// 起動時・設定再読み込み時に呼ぶ。ネットワークと SQLite 更新は専用スレッドで行う。
pub fn refresh_async(settings: AzureDevOpsSettings, notify: HWND, message: u32) -> bool {
    if !settings.enabled || settings.projects.is_empty() {
        return false;
    }
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let notify = notify.0 as isize;
    thread::spawn(move || {
        match http_client() {
            Ok(client) => {
                for project in settings
                    .projects
                    .iter()
                    .filter(|project| valid_project(project))
                {
                    let Ok(pat) = load_pat(&project.organization) else {
                        let _ = cache::record_project_error(
                            project,
                            "No PAT is saved for this organization.",
                        );
                        continue;
                    };
                    if let Err(error) = refresh_project(&client, project, &pat) {
                        crate::panic_log::record(&format!(
                            "azure devops: refresh {}/{} failed: {error}",
                            project.organization, project.project
                        ));
                        let _ = cache::record_project_error(project, &error);
                    }
                }
            }
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: could not initialize refresh client: {error}"
            )),
        }
        REFRESHING.store(false, Ordering::Release);
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
        }
    });
    true
}

/// `az wit ` の検索をバックグラウンドで実行する。結果は ID ごとに保持し、
/// 呼び出し側が最新 ID と一致したものだけを表示する。
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
                for project in settings
                    .projects
                    .iter()
                    .filter(|project| valid_project(project) && project.include_work_items)
                {
                    let Ok(pat) = load_pat(&project.organization) else {
                        failures.push(format!("{}: no PAT", project.organization));
                        continue;
                    };
                    match fetch_work_items(&client, project, &pat, &query) {
                        Ok(mut found) => {
                            if let Err(error) = cache::cache_work_item_candidates(&found) {
                                crate::panic_log::record(&format!(
                                    "azure devops: could not cache work items: {error}"
                                ));
                            }
                            results.append(&mut found);
                        }
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
        let mut pending = pending_work_items()
            .lock()
            .expect("work item result lock poisoned");
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
    pending_work_items().lock().ok()?.remove(&request_id)
}

fn pending_work_items() -> &'static Mutex<HashMap<u32, WorkItemReply>> {
    static PENDING: OnceLock<Mutex<HashMap<u32, WorkItemReply>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}
