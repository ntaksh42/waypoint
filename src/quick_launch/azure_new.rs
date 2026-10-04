//! 作成種別 → Project → リポジトリの選択。入力経路では I/O を行わない。

use super::{Action, AzureMeta, Entry};
use crate::azure_devops::{
    Kind, creation_project_url, encode_url_segment, work_item_type_names, work_item_type_style,
};
use crate::config::{AzureDevOpsProject, AzureDevOpsSettings};

#[derive(Debug, Clone, Default)]
pub(crate) struct CreationMenu {
    types: Vec<CreationType>,
    setup: Option<Entry>,
}

#[derive(Debug, Clone)]
struct CreationType {
    key: String,
    choice: Entry,
    projects: Vec<CreationProject>,
}

#[derive(Debug, Clone)]
struct CreationProject {
    scope: String,
    choice: Entry,
    repositories: Vec<Entry>,
}

pub(super) fn entries(settings: &AzureDevOpsSettings) -> CreationMenu {
    entries_with_types(settings, work_item_type_names)
}

fn entries_with_types(
    settings: &AzureDevOpsSettings,
    types: impl Fn(&str, &str) -> Vec<String>,
) -> CreationMenu {
    if !settings.enabled {
        return setup_menu("Enable Azure DevOps and add projects in Settings");
    }
    let mut projects = settings.projects.iter().collect::<Vec<_>>();
    projects.sort_by_key(|project| project.priority);
    let mut menu = CreationMenu::default();
    for project in projects {
        if project.organization.trim().is_empty() || project.project.trim().is_empty() {
            continue;
        }
        let base = creation_project_url(project);
        let scope = format!("{}/{}", project.organization.trim(), project.project.trim());
        if project.include_work_items {
            let mut names = types(&project.organization, &project.project);
            let unsynced = names.is_empty();
            if unsynced {
                // URL を開くだけなので、アプリの認証切れでも入口を残す。
                names = ["Bug", "Task", "User Story"].map(str::to_string).to_vec();
            }
            for name in names {
                let choice = entry(
                    project.project.trim().into(),
                    format!(
                        "{} — Create {name}{}",
                        project.organization.trim(),
                        if unsynced {
                            " (standard type; availability varies)"
                        } else {
                            ""
                        }
                    ),
                    format!("{base}/_workitems/create/{}", encode_url_segment(&name)),
                    project,
                    Some(&name),
                );
                menu.add(
                    &name,
                    CreationProject {
                        scope: scope.clone(),
                        choice,
                        repositories: Vec::new(),
                    },
                );
            }
        }
        if project.include_pull_requests {
            let repositories = project
                .interest_repositories
                .iter()
                .map(|repo| repo.trim())
                .filter(|repo| !repo.is_empty())
                .map(|repository| {
                    entry(
                        repository.into(),
                        format!("{scope} — Create pull request"),
                        format!(
                            "{base}/_git/{}/pullrequestcreate",
                            encode_url_segment(repository)
                        ),
                        project,
                        None,
                    )
                })
                .collect::<Vec<_>>();
            let mut choice = entry(
                project.project.trim().into(),
                format!(
                    "{} — {}",
                    project.organization.trim(),
                    if repositories.is_empty() {
                        "Choose a repository in browser"
                    } else {
                        "Choose a repository"
                    }
                ),
                format!("{base}/_git"),
                project,
                None,
            );
            if !repositories.is_empty() {
                choice.path.clear();
                choice.action = Action::ReplaceQuery(format!("az new pr {scope} "));
            }
            menu.add(
                "pr",
                CreationProject {
                    scope,
                    choice,
                    repositories,
                },
            );
        }
    }
    if menu.types.is_empty() {
        return setup_menu("Add projects and enable Work Items or PRs in Settings");
    }
    menu.types.sort_by_key(|kind| match kind.key.as_str() {
        "bug" => (0, String::new()),
        "task" => (1, String::new()),
        "user story" => (2, String::new()),
        "pr" => (3, String::new()),
        _ => (4, kind.key.clone()),
    });
    menu
}

impl CreationMenu {
    fn add(&mut self, name: &str, project: CreationProject) {
        let key = name.to_lowercase();
        if let Some(kind) = self.types.iter_mut().find(|kind| kind.key == key) {
            kind.projects.push(project);
            return;
        }
        let mut choice = project.choice.clone();
        choice.name = if key == "pr" {
            "Pull request".into()
        } else {
            name.into()
        };
        choice.breadcrumb = "Choose a project".into();
        choice.path.clear();
        choice.action = Action::ReplaceQuery(format!("az new {key} "));
        self.types.push(CreationType {
            key,
            choice,
            projects: vec![project],
        });
    }
}

fn setup_menu(detail: &str) -> CreationMenu {
    CreationMenu {
        types: Vec::new(),
        setup: Some(Entry {
            name: "Configure Azure DevOps".into(),
            breadcrumb: detail.into(),
            path: String::new(),
            action: Action::OpenSettings,
            azure: None,
            branch: None,
        }),
    }
}

fn entry(
    name: String,
    breadcrumb: String,
    url: String,
    project: &AzureDevOpsProject,
    work_item_type: Option<&str>,
) -> Entry {
    Entry {
        name,
        breadcrumb,
        path: url.clone(),
        action: Action::OpenUrl(url),
        azure: Some(AzureMeta {
            kind: if work_item_type.is_some() {
                Kind::WorkItem
            } else {
                Kind::PullRequest
            },
            work_item_style: work_item_type.and_then(|name| {
                work_item_type_style(&project.organization, &project.project, name)
            }),
            work_item_type: work_item_type.map(str::to_string),
            status: String::new(),
            is_mine: false,
            needs_my_review: false,
            is_draft: false,
        }),
        branch: None,
    }
}

/// 空白を含む型名・Project 名も選択時に補完した文字列で識別する。
fn after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}

pub(super) fn search<'a>(menu: &'a CreationMenu, query: &str) -> Vec<&'a Entry> {
    if let Some(setup) = &menu.setup {
        return vec![setup];
    }
    let query = query.trim().to_lowercase();
    let selected = menu
        .types
        .iter()
        .filter_map(|kind| {
            after(&query, &kind.key)
                .or_else(|| {
                    (kind.key == "user story")
                        .then(|| after(&query, "story"))
                        .flatten()
                })
                .map(|rest| (kind, rest))
        })
        .max_by_key(|(kind, _)| kind.key.len());
    let Some((kind, rest)) = selected else {
        return menu
            .types
            .iter()
            .filter(|kind| matches(&kind.choice, &query))
            .map(|kind| &kind.choice)
            .collect();
    };
    if kind.key == "pr"
        && let Some((project, rest)) = kind
            .projects
            .iter()
            .filter(|project| !project.repositories.is_empty())
            .filter_map(|project| {
                after(rest, &project.scope.to_lowercase()).map(|rest| (project, rest))
            })
            .max_by_key(|(project, _)| project.scope.len())
    {
        return project
            .repositories
            .iter()
            .filter(|entry| matches(entry, rest))
            .collect();
    }
    kind.projects
        .iter()
        .filter(|project| matches(&project.choice, rest))
        .map(|project| &project.choice)
        .collect()
}

fn matches(entry: &Entry, query: &str) -> bool {
    let text = format!("{} {}", entry.name, entry.breadcrumb).to_lowercase();
    query.split_whitespace().all(|term| text.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> AzureDevOpsSettings {
        let project = AzureDevOpsProject {
            organization: "org".into(),
            project: "Low priority".into(),
            aliases: Vec::new(),
            priority: 10,
            include_pull_requests: true,
            include_pipelines: false,
            include_work_items: true,
            interest_areas: Vec::new(),
            interest_iterations: Vec::new(),
            interest_repositories: vec!["repo #1".into(), " ".into()],
        };
        AzureDevOpsSettings {
            enabled: true,
            projects: vec![
                project.clone(),
                AzureDevOpsProject {
                    project: "High priority".into(),
                    priority: 0,
                    ..project
                },
            ],
        }
    }

    fn next_query(entry: &Entry) -> &str {
        let Action::ReplaceQuery(query) = &entry.action else {
            panic!("selection should advance the menu");
        };
        query.strip_prefix("az new ").unwrap()
    }

    #[test]
    fn type_then_project_opens_the_creation_url_in_priority_order() {
        let menu = entries_with_types(&settings(), |_, _| {
            vec![
                "Bug".into(),
                "Task".into(),
                "User Story".into(),
                "Custom / Type".into(),
            ]
        });
        let types = search(&menu, "");
        assert_eq!(
            types
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Bug", "Task", "User Story", "Pull request", "Custom / Type"]
        );
        let projects = search(&menu, next_query(types[2]));
        assert_eq!(
            projects
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["High priority", "Low priority"]
        );
        assert_eq!(
            projects[0].path,
            "https://dev.azure.com/org/High%20priority/_workitems/create/User%20Story"
        );
        assert_eq!(
            projects[0].action,
            Action::OpenUrl(projects[0].path.clone())
        );
        assert_eq!(search(&menu, "story")[0].path, projects[0].path);
        assert_eq!(search(&menu, "bug low")[0].name, "Low priority");
        assert_eq!(
            search(&menu, "custom / type")[0].path,
            "https://dev.azure.com/org/High%20priority/_workitems/create/Custom%20%2F%20Type"
        );
        assert_eq!(search(&menu, "bu")[0].name, "Bug");
        assert!(search(&menu, "missing").is_empty());
    }

    #[test]
    fn pull_request_selects_project_then_repository_without_opening_early() {
        let menu = entries_with_types(&settings(), |_, _| vec!["Problem".into()]);
        let projects = search(&menu, "pr");
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "High priority");
        let repositories = search(&menu, next_query(projects[0]));
        assert_eq!(repositories.len(), 1);
        assert_eq!(repositories[0].name, "repo #1");
        assert_eq!(
            repositories[0].path,
            "https://dev.azure.com/org/High%20priority/_git/repo%20%231/pullrequestcreate"
        );
        assert_eq!(
            repositories[0].action,
            Action::OpenUrl(repositories[0].path.clone())
        );
        assert_eq!(search(&menu, "pr org/high priority repo").len(), 1);
    }

    #[test]
    fn synced_types_only_offer_projects_that_support_them() {
        let menu = entries_with_types(&settings(), |_, project| {
            if project == "High priority" {
                vec!["Task".into()]
            } else {
                vec!["Bug".into()]
            }
        });
        assert_eq!(search(&menu, "bug").len(), 1);
        assert_eq!(search(&menu, "bug")[0].name, "Low priority");
    }

    #[test]
    fn unsynced_types_and_missing_repositories_have_browser_entries() {
        let mut settings = settings();
        for project in &mut settings.projects {
            project.interest_repositories.clear();
        }
        let menu = entries_with_types(&settings, |_, _| Vec::new());
        assert_eq!(search(&menu, "").len(), 4);
        for kind in ["bug", "task", "story", "pr"] {
            assert_eq!(search(&menu, kind).len(), 2);
        }
        let projects = search(&menu, "pr");
        assert_eq!(
            projects[0].path,
            "https://dev.azure.com/org/High%20priority/_git"
        );
        assert!(matches!(projects[0].action, Action::OpenUrl(_)));
        settings.enabled = false;
        assert_eq!(
            search(&entries_with_types(&settings, |_, _| Vec::new()), "bug")[0].action,
            Action::OpenSettings
        );
        settings.enabled = true;
        for project in &mut settings.projects {
            project.include_pull_requests = false;
            project.include_work_items = false;
        }
        assert_eq!(
            search(&entries_with_types(&settings, |_, _| Vec::new()), "")[0].action,
            Action::OpenSettings
        );
    }

    #[test]
    fn index_routes_new_commands_without_live_search() {
        let index = super::super::Index {
            azure_new: entries_with_types(&settings(), |_, _| vec!["User Story".into()]),
            ..Default::default()
        };
        assert_eq!(index.search("az new").len(), 2);
        assert_eq!(index.search("az new story").len(), 2);
        assert_eq!(index.search("az new pr org/High priority").len(), 1);
        assert_eq!(super::super::azure_live_request("az new bug"), None);
    }
}
