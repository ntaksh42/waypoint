//! モードバッジ・種別色の判定。

use windows::Win32::Foundation::COLORREF;

use super::azure_tile::AzureTile;
use super::{ACCENT, TAG_TEXT, rgb};
use crate::azure_devops::Kind;
use crate::quick_launch::{Action, AzureMeta, Entry};

/// モードバッジの背景色。プレフィックスごとに見分けは付けるが、
/// 彩度・明度は揃えた寒色2トーンに統一し、原色の乱立を避ける。
pub(super) fn badge_color(badge: &str) -> COLORREF {
    match badge {
        "WINDOWS" | "APPS" | "TERMINAL" | "CLAUDE CODE" | "CODEX" | "SESSIONS" => {
            rgb(143, 168, 118)
        } // 緑寄りの寒色
        "BOOKMARKS" | "HISTORY" | "FILES" | "TABS" | "AZURE DEVOPS" | "WEB" => rgb(95, 157, 176), // 青寄りの寒色
        // kill は確認なしの即時破壊操作 (FR-9.15.2) なので、誤入力に気付けるよう
        // 他プレフィックスの寒色2トーンと区別できる警告色にする
        "KILL" => rgb(196, 92, 80),
        _ => ACCENT,
    }
}

pub(super) fn shows_live_search_hint(badge: Option<&str>) -> bool {
    badge == Some("AZURE DEVOPS")
}

/// 候補行の右端に出す種別タグ。アイコンだけでは見分けにくい
/// (フォルダ・ファイル・URL がどれも似た見た目になる) ため、文字で補う。
/// `az ` モードの Azure DevOps 候補は URL から PR / Work Item 等を判定する。
pub(super) fn action_tag(action: &Action, badge: Option<&str>, path: &str) -> &'static str {
    if let Some(kind) = azure_icon_kind(badge, path) {
        return match kind {
            AzureIconKind::PullRequest => "PR",
            AzureIconKind::WorkItem => "Work Item",
            AzureIconKind::Pipeline => "Pipeline",
            AzureIconKind::Project => "Project",
        };
    }
    match action {
        Action::OpenFolder(_) => "Folder",
        Action::FocusWindow(_) => "Window",
        Action::FocusBrowserTab(_) => "Tab",
        Action::OpenUrl(_) => "Link",
        Action::OpenWithDefaultHandler => "File",
        Action::LaunchApp => "App",
        Action::OpenInTerminal => "Terminal",
        Action::OpenInEditor(_) => "Editor",
        Action::OpenClaudeCode(_) => "Claude Code",
        Action::OpenCodex => "Codex",
        Action::ResumeAgentSession(agent, _) => agent.label(),
        // `cc ` のフォルダ候補は path を持つ ReplaceQuery (draw_row.rs 参照)
        Action::ReplaceQuery(_) if !path.is_empty() => "Folder",
        Action::ReplaceQuery(_)
        | Action::AzureOptimize
        | Action::OpenSettings
        | Action::OpenHelp => "Command",
        Action::AzureLiveWorkItemSearch(_)
        | Action::AzureLivePullRequestSearch { .. }
        | Action::AzureLivePipelineSearch { .. } => "Search",
        Action::WebSearch => "Web",
        Action::KillProcess(_) => "Kill",
    }
}

/// 種別タグの文字色。kill は確認なしの即時破壊操作 (FR-9.15.2) なので、
/// モードバッジと同じ警告色にして誤操作に気付けるようにする。
pub(super) fn action_tag_color(action: &Action) -> COLORREF {
    match action {
        Action::KillProcess(_) => badge_color("KILL"),
        _ => TAG_TEXT,
    }
}

/// 選択行の右端に種別タグの代わりに出す、Enter で起きることの説明。
pub(super) fn action_verb(action: &Action) -> &'static str {
    match action {
        Action::FocusWindow(_) | Action::FocusBrowserTab(_) => "Switch \u{21B5}",
        Action::ReplaceQuery(_) => "Complete \u{21B5}",
        Action::AzureLiveWorkItemSearch(_)
        | Action::AzureLivePullRequestSearch { .. }
        | Action::AzureLivePipelineSearch { .. }
        | Action::WebSearch => "Search \u{21B5}",
        Action::AzureOptimize => "Run \u{21B5}",
        Action::LaunchApp | Action::OpenClaudeCode(_) | Action::OpenCodex => "Launch \u{21B5}",
        Action::KillProcess(_) => "Kill \u{21B5}",
        Action::ResumeAgentSession(..) => "Resume \u{21B5}",
        Action::OpenFolder(_)
        | Action::OpenUrl(_)
        | Action::OpenWithDefaultHandler
        | Action::OpenInTerminal
        | Action::OpenInEditor(_)
        | Action::OpenSettings
        | Action::OpenHelp => "Open \u{21B5}",
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

fn kind_style(kind: AzureIconKind) -> (COLORREF, &'static str) {
    match kind {
        AzureIconKind::PullRequest => (rgb(191, 90, 242), "⇄"), // 紫
        AzureIconKind::WorkItem => (rgb(0, 120, 212), "◆"),     // 青
        AzureIconKind::Pipeline => (rgb(52, 199, 89), "▶"),     // 緑
        AzureIconKind::Project => (rgb(48, 176, 199), "▦"),     // シアン
    }
}

/// Work Item Type ごとの色 (Azure DevOps 本家の配色に合わせる) と記号。
/// 未知の Type (プロセステンプレート独自の型など) は従来の Work Item の青。
pub(super) fn work_item_style(work_item_type: &str) -> (COLORREF, &'static str) {
    match work_item_type.to_ascii_lowercase().as_str() {
        "bug" => (rgb(204, 41, 61), "✱"),      // 赤
        "task" => (rgb(216, 169, 0), "✓"),     // 黄
        "feature" => (rgb(119, 59, 147), "★"), // 紫
        "epic" => (rgb(255, 123, 0), "♛"),     // 橙
        _ => kind_style(AzureIconKind::WorkItem),
    }
}

/// 自分のレビュー待ち (橙) > Pipeline の失敗 (赤) > Draft (灰) の順に 1 つだけ。
pub(super) fn marker_color(meta: &AzureMeta) -> Option<COLORREF> {
    if meta.needs_my_review {
        Some(rgb(255, 149, 0))
    } else if meta.is_failed_pipeline() {
        Some(rgb(229, 72, 77))
    } else if meta.is_draft {
        Some(rgb(142, 142, 147))
    } else {
        None
    }
}

/// `az ` モードの行頭アイコン。候補が種別・状態 (`Entry::azure`) を持てば
/// それを使い、持たなければ従来どおり URL から種別だけを推定する。
pub(super) fn azure_tile(badge: Option<&str>, entry: &Entry) -> Option<AzureTile> {
    let url_kind = azure_icon_kind(badge, &entry.path)?;
    let Some(meta) = &entry.azure else {
        let (color, glyph) = kind_style(url_kind);
        return Some(AzureTile {
            color,
            glyph,
            marker: None,
            closed: false,
        });
    };
    let (color, glyph) = match (meta.kind, meta.work_item_type.as_deref()) {
        (Kind::WorkItem, Some(work_item_type)) => work_item_style(work_item_type),
        (Kind::WorkItem, None) => kind_style(AzureIconKind::WorkItem),
        (Kind::PullRequest, _) => kind_style(AzureIconKind::PullRequest),
        (Kind::Pipeline, _) => kind_style(AzureIconKind::Pipeline),
        (Kind::Project, _) => kind_style(AzureIconKind::Project),
    };
    Some(AzureTile {
        color,
        glyph,
        marker: marker_color(meta),
        closed: meta.is_closed(),
    })
}
