//! 自動最適化の実行・保存と完了結果。ネットワークと保存は UI スレッド外で行う。

use super::sync::{RunningFlag, apply_optimization, suggest_priorities_async};
use crate::config::AzureDevOpsSettings;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::thread;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

static RESULT: Mutex<Option<(bool, String)>> = Mutex::new(None);

static OPTIMIZING: AtomicBool = AtomicBool::new(false);

/// `az optimize` 用。ネットワーク集計と保存を UI スレッド外で完結させ、
/// 成否を通知する。同じ最適化を同時に走らせない。
pub fn optimize_async(settings: AzureDevOpsSettings, notify: HWND, message: u32) -> bool {
    let Some(running) = RunningFlag::acquire(&OPTIMIZING) else {
        return false;
    };
    let notify = notify.0 as isize;
    thread::spawn(move || {
        // パニックしてもフラグを下ろすため、ワーカーの生存期間に紐づける。
        let _running = running;
        let result = suggest_priorities_async(settings)
            .recv()
            .map_err(|_| "Recent activity worker stopped unexpectedly.".to_string())
            .and_then(|result| result)
            .and_then(|activity| match crate::config::load() {
                crate::config::LoadOutcome::Loaded(mut config)
                | crate::config::LoadOutcome::Created(mut config) => {
                    let optimized = apply_optimization(
                        &mut config.settings.quick_launch.azure_devops,
                        &activity,
                    );
                    if optimized == 0 {
                        return Err(
                            "No recent work item activity found in the last 90 days.".to_string()
                        );
                    }
                    crate::config::save(&config)
                        .map_err(|error| format!("Could not save optimization: {error}"))?;
                    let failed = activity.iter().filter(|project| project.failed).count();
                    Ok((optimized, failed))
                }
                crate::config::LoadOutcome::Failed(error) => Err(error),
            });
        let result = result.map(|(optimized, failed)| format!(
            "Saved priorities and iteration filters for {optimized} project(s). {failed} failed project(s) kept unchanged."
        ));
        let saved = result.is_ok();
        let description = match &result {
            Ok(message) => message.clone(),
            Err(error) => format!("Optimization failed: {error}"),
        };
        crate::panic_log::record(&format!("azure devops: {description}"));
        *RESULT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((saved, description));
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
        }
    });
    true
}

pub(crate) fn take_optimization_result() -> Option<(bool, String)> {
    RESULT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}
