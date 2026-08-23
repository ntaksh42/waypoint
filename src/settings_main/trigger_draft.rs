//! トリガー設定ダイアログの状態と、Azure DevOps プロジェクト行の
//! パース・整形・選択反映ロジック。

use waypoint::config::{AzureDevOpsProject, Config};
use waypoint::hotkey_capture;

type AzureProjectLoad = (String, Result<Vec<String>, String>);

pub(super) struct TriggerDraft {
    pub(super) middle_click: bool,
    pub(super) hotkey: String,
    pub(super) excluded_processes: String,
    pub(super) quick_launch_hotkey: String,
    pub(super) include_recent_folders: bool,
    pub(super) include_frequent_folders: bool,
    pub(super) include_open_windows: bool,
    pub(super) include_bookmarks: bool,
    pub(super) include_browser_history: bool,
    pub(super) include_apps: bool,
    pub(super) azure_enabled: bool,
    pub(super) azure_projects: String,
    pub(super) include_everything: bool,
    pub(super) search_paths: bool,
    pub(super) visible_results: usize,
    pub(super) error: Option<String>,
    /// キー入力から記録中の欄 (FR-6.8.1) 。
    pub(super) recording: Option<HotkeyField>,
}

/// 多数の Azure DevOps プロジェクトを検索して選ぶ専用画面の状態。
pub(super) struct AzureProjectPicker {
    pub(super) watched_projects: String,
    pub(super) organization: String,
    pub(super) pat: String,
    pub(super) filter: String,
    pub(super) show_selected_only: bool,
    pub(super) available_projects: Vec<String>,
    pub(super) selected_projects: std::collections::BTreeSet<String>,
    pub(super) loaded_organization: String,
    pub(super) loader: Option<std::sync::mpsc::Receiver<AzureProjectLoad>>,
    pub(super) loading: bool,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
}

/// ホットキーを持つ欄。記録先の指定に使う。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HotkeyField {
    Menu,
    QuickLaunch,
}

impl TriggerDraft {
    pub(super) fn from_config(config: &Config) -> Self {
        let trigger = &config.settings.trigger;
        let quick_launch = &config.settings.quick_launch;
        Self {
            middle_click: trigger.middle_click,
            hotkey: trigger.hotkey.clone(),
            excluded_processes: trigger.excluded_processes.join("\n"),
            quick_launch_hotkey: quick_launch.hotkey.clone(),
            include_recent_folders: quick_launch.include_recent_folders,
            include_frequent_folders: quick_launch.include_frequent_folders,
            include_open_windows: quick_launch.include_open_windows,
            include_bookmarks: quick_launch.include_bookmarks,
            include_browser_history: quick_launch.include_browser_history,
            include_apps: quick_launch.include_apps,
            azure_enabled: quick_launch.azure_devops.enabled,
            azure_projects: format_azure_projects(&quick_launch.azure_devops.projects),
            include_everything: quick_launch.include_everything,
            search_paths: quick_launch.search_paths,
            visible_results: quick_launch.visible_results,
            error: None,
            recording: None,
        }
    }

    pub(super) fn field_mut(&mut self, field: HotkeyField) -> &mut String {
        match field {
            HotkeyField::Menu => &mut self.hotkey,
            HotkeyField::QuickLaunch => &mut self.quick_launch_hotkey,
        }
    }
}

impl AzureProjectPicker {
    pub(super) fn new(trigger: &TriggerDraft) -> Self {
        let projects = parse_azure_projects(&trigger.azure_projects).unwrap_or_default();
        let organization = projects
            .first()
            .map(|project| project.organization.clone())
            .unwrap_or_default();
        Self {
            watched_projects: trigger.azure_projects.clone(),
            organization,
            pat: String::new(),
            filter: String::new(),
            show_selected_only: false,
            available_projects: Vec::new(),
            selected_projects: std::collections::BTreeSet::new(),
            loaded_organization: String::new(),
            loader: None,
            loading: false,
            status: None,
            error: None,
        }
    }

    /// PAT 入力欄または保存済み資格情報を使い、設定画面を止めずに一覧を取る。
    pub(super) fn start_load(&mut self) {
        let organization = self.organization.trim().to_string();
        if organization.is_empty() {
            self.error = Some("Organization is required.".to_string());
            return;
        }

        let pat = self.pat.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.loader = Some(receiver);
        self.loading = true;
        self.error = None;
        self.status = Some("Loading Azure DevOps projects...".to_string());
        std::thread::spawn(move || {
            let result = waypoint::azure_devops::list_projects(&organization, &pat);
            let _ = sender.send((organization, result));
        });
    }

    /// 非同期取得の完了を描画フレームで受け取る。
    pub(super) fn poll_load(&mut self) {
        let Some(receiver) = self.loader.as_ref() else {
            return;
        };
        let result = receiver.try_recv();
        let (organization, result) = match result {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.loader = None;
                self.loading = false;
                self.status = None;
                self.error = Some("Azure DevOps project loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.loader = None;
        self.loading = false;
        match result {
            Ok(projects) => match selected_azure_projects(&self.watched_projects, &organization) {
                Ok(selected) => {
                    self.selected_projects = projects
                        .iter()
                        .filter(|project| {
                            selected
                                .iter()
                                .any(|name| name.eq_ignore_ascii_case(project))
                        })
                        .cloned()
                        .collect();
                    self.available_projects = projects;
                    self.loaded_organization = organization;
                    self.error = None;
                    self.status = Some(format!(
                        "Loaded {} Azure DevOps projects.",
                        self.available_projects.len()
                    ));
                }
                Err(error) => self.error = Some(error),
            },
            Err(error) => {
                self.status = None;
                self.error = Some(error);
            }
        }
    }
}

/// ホットキー 1 欄。直接入力と、実際のキー入力からの記録 (FR-6.8.1) 。
pub(super) fn hotkey_row(ui: &mut eframe::egui::Ui, draft: &mut TriggerDraft, field: HotkeyField) {
    let recording = draft.recording == Some(field);
    ui.horizontal(|ui| {
        ui.add_enabled(
            !recording,
            eframe::egui::TextEdit::singleline(draft.field_mut(field)).desired_width(200.0),
        );
        let label = if recording { "Press keys..." } else { "Record" };
        if ui.button(label).clicked() {
            if recording {
                hotkey_capture::stop();
                draft.recording = None;
            } else if hotkey_capture::start() {
                draft.recording = Some(field);
                draft.error = None;
            } else {
                draft.error = Some("Could not capture keys. Type the hotkey instead.".to_string());
            }
        }
        if recording {
            ui.weak("Esc to cancel");
        }
    });
}

/// 記録中は毎フレーム結果を拾う。ウィンドウがフォーカスを失ったら、
/// 打鍵を握り潰したままにしないよう記録を打ち切る。
pub(super) fn poll_hotkey_capture(ctx: &eframe::egui::Context, draft: &mut TriggerDraft) {
    let Some(field) = draft.recording else {
        return;
    };
    if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
        hotkey_capture::stop();
        draft.recording = None;
        return;
    }
    // フックの結果は egui のイベントで届かないので、記録中は描画を回し続ける
    ctx.request_repaint();
    match hotkey_capture::poll() {
        Some(hotkey_capture::Captured::Spec(spec)) => {
            *draft.field_mut(field) = spec;
            draft.recording = None;
        }
        Some(hotkey_capture::Captured::Cancelled) => draft.recording = None,
        Some(hotkey_capture::Captured::Unsupported) => {
            draft.error = Some("That key cannot be used. Use A-Z, 0-9 or F1-F24.".to_string());
            draft.recording = None;
        }
        None => {}
    }
}

pub(super) fn format_azure_projects(projects: &[AzureDevOpsProject]) -> String {
    projects
        .iter()
        .map(|project| {
            format!(
                "{}/{} | {} | {} | {}",
                project.organization,
                project.project,
                project.aliases.join(", "),
                project.priority,
                format_azure_scopes(project)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_azure_scopes(project: &AzureDevOpsProject) -> String {
    [
        (project.include_pull_requests, "pr"),
        (project.include_pipelines, "pipelines"),
        (project.include_work_items, "wit"),
    ]
    .into_iter()
    .filter_map(|(included, label)| included.then_some(label))
    .collect::<Vec<_>>()
    .join(",")
}

pub(super) fn parse_azure_projects(text: &str) -> Result<Vec<AzureDevOpsProject>, String> {
    let mut projects = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut columns = line.split('|').map(str::trim);
        let target = columns.next().unwrap_or_default();
        let aliases = columns.next().unwrap_or_default();
        let priority = columns.next().unwrap_or_default();
        let scopes = columns.next().unwrap_or_default();
        if columns.next().is_some() {
            return Err(format!(
                "Azure DevOps project line {} has too many columns.",
                line_number + 1
            ));
        }
        let Some((organization, project)) = target.split_once('/') else {
            return Err(format!(
                "Azure DevOps project line {} must be organization/project.",
                line_number + 1
            ));
        };
        let organization = organization.trim();
        let project = project.trim();
        if organization.is_empty() || project.is_empty() {
            return Err(format!(
                "Azure DevOps project line {} must name both organization and project.",
                line_number + 1
            ));
        }
        let key = format!(
            "{}/{}",
            organization.to_ascii_lowercase(),
            project.to_ascii_lowercase()
        );
        if !seen.insert(key) {
            return Err(format!(
                "Azure DevOps project line {} duplicates an earlier project.",
                line_number + 1
            ));
        }
        let priority = if priority.is_empty() {
            0
        } else {
            priority.parse().map_err(|_| {
                format!(
                    "Azure DevOps project line {} has an invalid priority.",
                    line_number + 1
                )
            })?
        };
        let scopes: Vec<_> = scopes
            .split(',')
            .map(|scope| scope.trim().to_ascii_lowercase())
            .filter(|scope| !scope.is_empty())
            .collect();
        let valid_scopes = ["pr", "pipelines", "wit"];
        if scopes
            .iter()
            .any(|scope| !valid_scopes.contains(&scope.as_str()))
        {
            return Err(format!(
                "Azure DevOps project line {} has an invalid sync scope.",
                line_number + 1
            ));
        }
        let all_scopes = scopes.is_empty();
        projects.push(AzureDevOpsProject {
            organization: organization.to_string(),
            project: project.to_string(),
            aliases: aliases
                .split(',')
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .map(str::to_string)
                .collect(),
            priority,
            include_pull_requests: all_scopes || scopes.iter().any(|scope| scope == "pr"),
            include_pipelines: all_scopes || scopes.iter().any(|scope| scope == "pipelines"),
            include_work_items: all_scopes || scopes.iter().any(|scope| scope == "wit"),
        });
    }
    Ok(projects)
}

/// 設定済みのうち、指定 Organization に属するプロジェクト名を抜き出す。
fn selected_azure_projects(
    text: &str,
    organization: &str,
) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(parse_azure_projects(text)?
        .into_iter()
        .filter(|project| project.organization.eq_ignore_ascii_case(organization))
        .map(|project| project.project)
        .collect())
}

/// チェック結果を設定本文へ反映する。他 Organization の設定と、選択済み項目の
/// aliases / priority はそのまま残す。
pub(super) fn merge_selected_azure_projects(
    text: &str,
    organization: &str,
    selected: &std::collections::BTreeSet<String>,
) -> Result<String, String> {
    let organization = organization.trim();
    if organization.is_empty() {
        return Err("Load an Azure DevOps project list first.".to_string());
    }
    let configured = parse_azure_projects(text)?;
    let mut updated: Vec<_> = configured
        .iter()
        .filter(|project| !project.organization.eq_ignore_ascii_case(organization))
        .cloned()
        .collect();

    for project_name in selected {
        if let Some(existing) = configured.iter().find(|project| {
            project.organization.eq_ignore_ascii_case(organization)
                && project.project.eq_ignore_ascii_case(project_name)
        }) {
            updated.push(existing.clone());
        } else {
            updated.push(AzureDevOpsProject {
                organization: organization.to_string(),
                project: project_name.clone(),
                aliases: Vec::new(),
                priority: 0,
                include_pull_requests: true,
                include_pipelines: true,
                include_work_items: true,
            });
        }
    }
    Ok(format_azure_projects(&updated))
}

pub(super) fn azure_project_count(draft: &TriggerDraft) -> usize {
    parse_azure_projects(&draft.azure_projects)
        .map(|projects| projects.len())
        .unwrap_or_default()
}
