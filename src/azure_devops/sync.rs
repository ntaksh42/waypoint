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
use super::api::{
    fetch_pull_requests_live, fetch_recent_activity_areas, fetch_work_items, http_client,
    refresh_project,
};
use super::cache;
use super::convert::{pull_request_cached_row_to_candidate, valid_project};
use super::credential::load_pat;

/// poisoned でも中身を取り出してロックする。
///
/// ライブ検索の結果置き場は `thread::spawn` した先から書き、UI スレッドから
/// 読む。どこか 1 つのスレッドがロック保持中にパニックすると、以後この
/// Mutex は永久に poisoned のままになる。`expect` で落とすと以降の Azure
/// 検索がすべてパニックし、`.ok()?` で握り潰すと結果が黙って返らなくなる
/// (どちらも 1 回のパニックが恒久的な機能停止になる)。
///
/// 中身は `HashMap<u32, Reply>` で、途中まで書けた状態でも壊れているのは
/// 高々 1 件ぶん。次のリクエストで上書きされるので、回収して続ける方が
/// 実害が小さい。`dynamic.rs` の `ResultSlot` と同じ方針。
fn lock_recovering<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Default)]
pub struct WorkItemReply {
    pub candidates: Vec<Candidate>,
    pub message: Option<String>,
}

/// PR のライブ検索結果。フィールドは `WorkItemReply` と同じ形だが、
/// `pending_work_items` と混ざらないよう独立した `reply_id` 空間を使う。
pub type PullRequestReply = WorkItemReply;

/// 同期を一つに直列化する。設定保存と手動更新が重なっても API と DB を競合させない。
pub(crate) static REFRESHING: AtomicBool = AtomicBool::new(false);

/// 起動時・設定再読み込み時に呼ぶ。ネットワークと SQLite 更新は専用スレッドで行う。
///
/// 監視プロジェクトはお互いに独立な読み取り・書き込みなので、`az wit` /
/// `az pr` のライブ検索と同じく `thread::scope` で並列に投げる (実測で
/// プロジェクト数が増えるほど直列実行の合計待ち時間が積み上がっていた)。
/// SQLite への書き込みはプロジェクトごとに別接続で行うため、同時書き込みは
/// ファイルロックにより自動的に順番待ちされるだけで安全。
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
                let targets: Vec<_> = settings
                    .projects
                    .iter()
                    .filter(|project| valid_project(project))
                    .collect();
                thread::scope(|scope| {
                    for project in &targets {
                        let client = &client;
                        scope.spawn(move || {
                            let Ok(pat) = load_pat(&project.organization) else {
                                let _ = cache::record_project_error(
                                    project,
                                    "No PAT is saved for this organization.",
                                );
                                return;
                            };
                            if let Err(error) = refresh_project(client, project, &pat) {
                                crate::panic_log::record(&format!(
                                    "azure devops: refresh {}/{} failed: {error}",
                                    project.organization, project.project
                                ));
                                let _ = cache::record_project_error(project, &error);
                            }
                        });
                    }
                });
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
                let outcomes: Vec<(
                    &crate::config::AzureDevOpsProject,
                    Result<Vec<Candidate>, String>,
                )> = thread::scope(|scope| {
                    let handles: Vec<_> = targets
                        .iter()
                        .map(|project| {
                            let client = &client;
                            let query = &query;
                            scope.spawn(move || {
                                let outcome = match load_pat(&project.organization) {
                                    Ok(pat) => fetch_work_items(client, project, &pat, query),
                                    Err(_) => Err(format!("{}: no PAT", project.organization)),
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
                let jobs: Vec<(&crate::config::AzureDevOpsProject, &'static str)> = targets
                    .iter()
                    .flat_map(|project| statuses.iter().map(move |status| (*project, *status)))
                    .collect();
                let outcomes: Vec<PullRequestFetchOutcome> = thread::scope(|scope| {
                    let handles: Vec<_> = jobs
                        .iter()
                        .map(|&(project, status)| {
                            let client = &client;
                            scope.spawn(move || {
                                let outcome = match load_pat(&project.organization) {
                                    Ok(pat) => fetch_pull_requests_live(
                                        client, project, &pat, status,
                                    )
                                    .map(|rows| {
                                        rows.iter()
                                            .map(|row| {
                                                pull_request_cached_row_to_candidate(project, row)
                                            })
                                            .collect()
                                    }),
                                    Err(_) => Err(format!("{}: no PAT", project.organization)),
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
            let outcomes: Vec<(
                &crate::config::AzureDevOpsProject,
                Result<Vec<String>, String>,
            )> = thread::scope(|scope| {
                let handles: Vec<_> = targets
                    .iter()
                    .map(|project| {
                        let client = &client;
                        scope.spawn(move || {
                            let outcome = match load_pat(&project.organization) {
                                Ok(pat) => fetch_recent_activity_areas(client, project, &pat),
                                Err(_) => Err(format!("{}: no PAT", project.organization)),
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

    /// poisoned な Mutex でも中身を取り出して続行する。
    ///
    /// ライブ検索の結果置き場はワーカースレッドが書き UI スレッドが読む。
    /// 1 スレッドのパニックで以後の Azure 検索が全滅しないことを固定する。
    #[test]
    fn lock_recovering_survives_a_poisoned_mutex() {
        use std::sync::{Arc, Mutex};

        let mutex = Arc::new(Mutex::new(vec![1u32, 2, 3]));

        // ロックを保持したままパニックさせて poisoned にする
        let poisoner = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().expect("最初の取得は成功する");
            panic!("ワーカーがロック保持中に落ちた");
        })
        .join();
        assert!(mutex.is_poisoned(), "前提: poisoned になっていること");

        // expect なら落ちる状況でも、中身を取り出して続けられる
        let mut guard = super::lock_recovering(&mutex);
        assert_eq!(*guard, vec![1, 2, 3], "poisoned でも中身は読める");
        guard.push(4);
        drop(guard);

        // 2 回目以降も同じく使える (恒久的な機能停止にならない)
        assert_eq!(*super::lock_recovering(&mutex), vec![1, 2, 3, 4]);
    }
}
