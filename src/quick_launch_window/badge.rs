//! モードバッジ・種別色の判定。

use windows::Win32::Foundation::COLORREF;

use super::{ACCENT, rgb};
use crate::quick_launch::Action;

/// モードバッジの背景色。プレフィックスごとに見分けは付けるが、
/// 彩度・明度は揃えた寒色2トーンに統一し、原色の乱立を避ける。
pub(super) fn badge_color(badge: &str) -> COLORREF {
    match badge {
        "WINDOWS" | "APPS" => rgb(143, 168, 118), // 緑寄りの寒色
        "BOOKMARKS" | "HISTORY" | "FILES" | "AZURE DEVOPS" => rgb(95, 157, 176), // 青寄りの寒色
        _ => ACCENT,
    }
}

/// 候補のアクション種別を表す色。種別の判別はアイコン自体の形が担うため、
/// 背景色は原則アクセント 1 色に寄せて静かにする。
pub(super) fn action_color(action: &Action) -> COLORREF {
    match action {
        Action::FocusWindow(_) | Action::LaunchApp => badge_color("WINDOWS"),
        Action::OpenFolder(_)
        | Action::OpenUrl(_)
        | Action::OpenWithDefaultHandler
        | Action::ReplaceQuery(_) => ACCENT,
    }
}

/// Azure DevOps 検索で URL から判定できる候補種別。通常の URL 検索では
/// favicon を優先するため、`az ` モード中だけこのアイコンを使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AzureIconKind {
    PullRequest,
    WorkItem,
    Pipeline,
    Project,
}

pub(super) fn azure_icon_kind(badge: Option<&str>, path: &str) -> Option<AzureIconKind> {
    if badge != Some("AZURE DEVOPS") || !path.starts_with("https://dev.azure.com/") {
        return None;
    }
    if path.contains("/pullrequest/") {
        Some(AzureIconKind::PullRequest)
    } else if path.contains("/_workitems/edit/") {
        Some(AzureIconKind::WorkItem)
    } else if path.contains("/_build") {
        Some(AzureIconKind::Pipeline)
    } else {
        Some(AzureIconKind::Project)
    }
}

pub(super) fn azure_icon_color(kind: AzureIconKind) -> COLORREF {
    match kind {
        AzureIconKind::PullRequest => rgb(191, 90, 242), // 紫
        AzureIconKind::WorkItem => rgb(0, 120, 212),     // 青
        AzureIconKind::Pipeline => rgb(52, 199, 89),     // 緑
        AzureIconKind::Project => rgb(48, 176, 199),     // シアン
    }
}

pub(super) fn azure_icon_label(kind: AzureIconKind) -> &'static str {
    match kind {
        AzureIconKind::PullRequest => "⇄",
        AzureIconKind::WorkItem => "◆",
        AzureIconKind::Pipeline => "▶",
        AzureIconKind::Project => "▦",
    }
}
