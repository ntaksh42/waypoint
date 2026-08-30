//! 同期処理の共通ヘルパーと、監視プロジェクト全体のバックグラウンド更新。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::config::AzureDevOpsSettings;

use super::super::api::{http_client, refresh_project};
use super::super::auth_cache::OrganizationValues;
use super::super::cache;
use super::super::convert::valid_project;
use super::super::credential::load_pat;

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
pub(super) fn lock_recovering<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
                let pats = OrganizationValues::new(
                    targets.iter().map(|project| project.organization.as_str()),
                );
                thread::scope(|scope| {
                    for project in &targets {
                        let client = &client;
                        let pats = &pats;
                        scope.spawn(move || {
                            let Some(Ok(pat)) = pats.get_or_init(&project.organization, || {
                                load_pat(&project.organization)
                            }) else {
                                let _ = cache::record_project_error(
                                    project,
                                    "No PAT is saved for this organization.",
                                );
                                return;
                            };
                            if let Err(error) = refresh_project(client, project, pat) {
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

#[cfg(test)]
mod tests {
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
