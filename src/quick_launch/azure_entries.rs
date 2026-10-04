//! Azure DevOps の固定候補 (コマンド補完・ショートカット) と、キャッシュ候補の
//! 検索用 `Entry` への変換。

use super::{Action, Entry};

/// `az ` の直後に出すコマンド候補。`pr` / `wit` / `pipeline` / `project` は
/// 検索対象を持つサブコマンドなので選んでも検索欄を補完するだけだが、
/// `optimize` は検索を挟まない単一アクションなので、選んだ時点で
/// `AzureOptimize` を直接実行する (`az optimize` とフルタイプして
/// Enter した場合と同じ 1 手で済ませる — 補完してからもう一度 Enter する
/// 二度手間を避ける)。
pub(crate) fn azure_command_entries() -> &'static [Entry] {
    static ENTRIES: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(|| {
        let mut entries: Vec<Entry> = [
            (
                "az pr",
                "Search pull requests \u{00B7} mine needs-review waiting draft stale",
            ),
            (
                "az wit",
                "Search work items \u{00B7} #<id> for an exact match",
            ),
            (
                "az pipeline",
                "Search build pipelines \u{00B7} failed definitions",
            ),
            ("az project", "Open configured projects"),
        ]
        .into_iter()
        .map(|(name, breadcrumb)| Entry {
            azure: None,
            name: name.to_string(),
            breadcrumb: breadcrumb.to_string(),
            path: String::new(),
            action: Action::ReplaceQuery(format!("{name} ")),
            branch: None,
        })
        .collect();
        entries.push(azure_suggest_entry());
        entries
    });
    &ENTRIES
}

/// `az ` の直後にコマンド候補より先に出す、入力を省くための PR 絞り込みショートカット。
/// リポジトリは設定画面で明示的に監視対象へ選んだものだけを使う。
pub(crate) fn azure_shortcut_entries(settings: &crate::config::AzureDevOpsSettings) -> Vec<Entry> {
    if !settings.enabled {
        return Vec::new();
    }
    let mut entries = vec![
        azure_shortcut(
            "My review requests",
            "Active PRs awaiting my vote",
            "az pr active needs-review ",
        ),
        azure_shortcut(
            "My active PRs",
            "Active PRs I authored",
            "az pr active author ",
        ),
        azure_shortcut(
            "Waiting for others",
            "My Active PRs with required reviewer feedback outstanding",
            "az pr active waiting ",
        ),
        azure_shortcut(
            "Stale PRs",
            "Active PRs created 14+ days ago",
            "az pr active stale ",
        ),
    ];
    for project in &settings.projects {
        if !project.include_pull_requests {
            continue;
        }
        for repository in &project.interest_repositories {
            let repository = repository.trim();
            if repository.is_empty() {
                continue;
            }
            entries.push(azure_shortcut(
                &format!("{repository} — review requests"),
                &format!(
                    "{} / {}",
                    project.organization.trim(),
                    project.project.trim()
                ),
                &format!("az pr active reviewer {repository} "),
            ));
            entries.push(azure_shortcut(
                &format!("{repository} — my PRs"),
                &format!(
                    "{} / {}",
                    project.organization.trim(),
                    project.project.trim()
                ),
                &format!("az pr active author {repository} "),
            ));
            entries.push(azure_shortcut(
                &format!("{repository} — waiting for others"),
                &format!(
                    "{} / {}",
                    project.organization.trim(),
                    project.project.trim()
                ),
                &format!("az pr active waiting {repository} "),
            ));
        }
    }
    entries
}

fn azure_shortcut(name: &str, breadcrumb: &str, query: &str) -> Entry {
    Entry {
        azure: None,
        name: name.to_string(),
        breadcrumb: breadcrumb.to_string(),
        path: String::new(),
        action: Action::ReplaceQuery(query.to_string()),
        branch: None,
    }
}

/// `az optimize`（`suggest` / `rank` でも入れる）に入ったときの唯一の確定候補。
/// 検索対象を持たないコマンドなので、選択すると直近アクティビティから
/// Project / Iteration の優先度をバックグラウンドで自動更新する。
pub(crate) fn azure_suggest_entry() -> Entry {
    Entry {
        azure: None,
        name: "az optimize".to_string(),
        breadcrumb: "Automatically rank projects & iterations from recent activity".to_string(),
        path: String::new(),
        action: Action::AzureOptimize,
        branch: None,
    }
}

pub(crate) fn azure_candidate_entry(candidate: crate::azure_devops::Candidate) -> Entry {
    Entry {
        azure: Some(super::AzureMeta::from_candidate(&candidate)),
        name: candidate.name,
        breadcrumb: if candidate.aliases.is_empty() {
            candidate.detail
        } else {
            format!("{} — {}", candidate.detail, candidate.aliases.join(" "))
        },
        path: candidate.url.clone(),
        action: Action::OpenUrl(candidate.url),
        branch: candidate.branch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_suggest_entry_triggers_the_suggest_priorities_action() {
        let entry = azure_suggest_entry();
        assert_eq!(entry.action, Action::AzureOptimize);
        assert!(entry.path.is_empty());
    }

    #[test]
    fn az_optimize_is_offered_among_the_top_level_command_completions() {
        assert!(
            azure_command_entries()
                .iter()
                .any(|entry| entry.name == "az optimize")
        );
    }

    #[test]
    fn azure_shortcuts_offer_personal_and_configured_repository_filters() {
        let settings = crate::config::AzureDevOpsSettings {
            enabled: true,
            projects: vec![crate::config::AzureDevOpsProject {
                organization: "org".into(),
                project: "project".into(),
                aliases: Vec::new(),
                priority: 0,
                include_pull_requests: true,
                include_pipelines: true,
                include_work_items: true,
                interest_areas: Vec::new(),
                interest_iterations: Vec::new(),
                interest_repositories: vec!["waypoint".into()],
            }],
        };
        let entries = azure_shortcut_entries(&settings);
        assert_eq!(entries.len(), 7);
        assert_eq!(entries[0].name, "My review requests");
        assert_eq!(
            entries[4].action,
            Action::ReplaceQuery("az pr active reviewer waypoint ".into())
        );
    }
}
