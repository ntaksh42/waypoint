//! Azure DevOps プロジェクト選択画面の状態とロジック。
//!
//! `projects` に設定済みプロジェクトをそのまま保持し、一覧から選んだ行を
//! `selected` (organization, project) で指し示す。詳細パネルの編集は
//! `projects` の該当エントリへ都度書き戻すので、Advanced テキスト DSL の
//! ような別表現との相互変換は発生しない。
//!
//! 「監視中の全プロジェクトを横断する優先度提案モーダル」は行の選択とは
//! 独立した状態を持つメソッド群のため `priority_suggestions.rs` の
//! `impl AzureProjectPicker` に分離してある (フィールド自体はここに残す —
//! `ui_azure_suggest.rs` 等が `picker.priority_suggestion_open` のように
//! フラットに触るので、ネストした子 struct にすると呼び出し側の書き換えが
//! 広範囲に及ぶ)。

mod async_load;
mod detail_load;
mod file_io;
mod priority_suggestions;

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::channel;
use std::thread;

use waypoint::azure_devops::{AreaNode, ProjectActivity};
use waypoint::config::{AzureDevOpsProject, Config};

use async_load::AsyncLoad;

#[cfg(test)]
pub(crate) use file_io::{decode_projects_json, encode_projects_json};

type ProjectListLoad = (String, Result<Vec<String>, String>);
pub(super) type AreaLoad = ((String, String), Result<Vec<AreaNode>, String>);
type AreaSuggestionLoad = ((String, String), Result<Vec<(String, usize)>, String>);
type RepositoryLoad = ((String, String), Result<Vec<String>, String>);

/// Azure DevOps 連携の設定画面。有効化トグルと監視プロジェクトの選択・
/// 詳細編集を 1 つの画面にまとめる (以前は「有効化トグルだけの入口画面」
/// →「Choose watched projects...」で開く別画面、の 2 段構成だったが、
/// それぞれ独立した OK/Apply を持つため保存漏れを招きやすかった)。
pub(super) struct AzureProjectPicker {
    pub(super) enabled: bool,
    pub(super) projects: Vec<AzureDevOpsProject>,
    pub(super) organization: String,
    pub(super) pat: String,
    pub(super) filter: String,
    pub(super) show_selected_only: bool,
    pub(super) available_projects: Vec<String>,
    pub(super) loaded_organization: String,
    project_loader: AsyncLoad<ProjectListLoad>,
    pub(super) loading: bool,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
    /// 詳細パネルで開いている行 (organization, project)。
    pub(super) selected: Option<(String, String)>,
    /// 選択行の Area Path 階層。行を切り替えるたびに読み直す。
    pub(super) area_nodes: Vec<AreaNode>,
    area_loader: AsyncLoad<AreaLoad>,
    pub(super) area_loading: bool,
    pub(super) area_error: Option<String>,
    pub(super) area_filter: String,
    /// 自分に割り当てられた Work Item から集計した Area Path 候補 (パス, 件数)。
    /// 明示的にボタンを押した時だけ取得する。
    pub(super) area_suggestions: Vec<(String, usize)>,
    area_suggestion_loader: AsyncLoad<AreaSuggestionLoad>,
    pub(super) area_suggestion_loading: bool,
    pub(super) area_suggestion_error: Option<String>,
    /// 選択行のリポジトリ名一覧。行を切り替えるたびに読み直す。
    pub(super) repositories: Vec<String>,
    repository_loader: AsyncLoad<RepositoryLoad>,
    pub(super) repository_loading: bool,
    pub(super) repository_error: Option<String>,
    pub(super) repository_filter: String,
    /// 編集中のテキスト欄 (aliases / priority)。行ごとに保持し、
    /// フォーカス移動時に確定して `projects` へ書き戻す。
    pub(super) aliases_text: String,
    pub(super) priority_text: String,
    /// 直近アクティビティ (アサイン + メンション) からの Project / Area 優先度提案。
    /// 監視中の全プロジェクトを横断するため、行の選択とは独立したモーダルで表示する。
    pub(super) priority_suggestions: Vec<ProjectActivity>,
    priority_suggestion_loader: AsyncLoad<Result<Vec<ProjectActivity>, String>>,
    pub(super) priority_suggestion_loading: bool,
    pub(super) priority_suggestion_error: Option<String>,
    /// 提案モーダルが開いているか、フィルタ文字列、チェック済み (organization, project) の集合。
    pub(super) priority_suggestion_open: bool,
    pub(super) priority_suggestion_filter: String,
    pub(super) priority_suggestion_checked: BTreeSet<(String, String)>,
    /// 提案モーダルでツリー展開中のプロジェクト (一度に 1 つ、アコーディオン式)。
    pub(super) priority_suggestion_expanded: Option<(String, String)>,
    /// 展開して取得済みの Area ツリー。プロジェクトを切り替えても再取得しない。
    pub(super) priority_suggestion_area_trees: HashMap<(String, String), Vec<AreaNode>>,
    pub(super) priority_suggestion_area_loading: Option<(String, String)>,
    pub(super) priority_suggestion_area_error: Option<String>,
    /// 展開中の Area ツリーの絞り込み。ツリーは数百件になることがあり、
    /// スクロールだけでは目的の Area に届かない。展開するプロジェクトを
    /// 切り替えたら消す (別ツリーに前の絞り込みが残ると 0 件に見える)。
    pub(super) priority_suggestion_area_filter: String,
    priority_suggestion_area_loader: AsyncLoad<AreaLoad>,
}

impl AzureProjectPicker {
    pub(super) fn from_config(config: &Config) -> Self {
        let azure_devops = &config.settings.quick_launch.azure_devops;
        let mut picker = Self::with_projects(azure_devops.projects.clone());
        picker.enabled = azure_devops.enabled;
        picker
    }

    #[cfg(test)]
    pub(super) fn new(projects: Vec<AzureDevOpsProject>) -> Self {
        Self::with_projects(projects)
    }

    fn with_projects(projects: Vec<AzureDevOpsProject>) -> Self {
        let organization = projects
            .first()
            .map(|project| project.organization.clone())
            .unwrap_or_default();
        Self {
            enabled: true,
            projects,
            organization,
            pat: String::new(),
            filter: String::new(),
            show_selected_only: false,
            available_projects: Vec::new(),
            loaded_organization: String::new(),
            project_loader: AsyncLoad::default(),
            loading: false,
            status: None,
            error: None,
            selected: None,
            area_nodes: Vec::new(),
            area_loader: AsyncLoad::default(),
            area_loading: false,
            area_error: None,
            area_filter: String::new(),
            area_suggestions: Vec::new(),
            area_suggestion_loader: AsyncLoad::default(),
            area_suggestion_loading: false,
            area_suggestion_error: None,
            repositories: Vec::new(),
            repository_loader: AsyncLoad::default(),
            repository_loading: false,
            repository_error: None,
            repository_filter: String::new(),
            aliases_text: String::new(),
            priority_text: String::new(),
            priority_suggestions: Vec::new(),
            priority_suggestion_loader: AsyncLoad::default(),
            priority_suggestion_loading: false,
            priority_suggestion_error: None,
            priority_suggestion_open: false,
            priority_suggestion_filter: String::new(),
            priority_suggestion_checked: BTreeSet::new(),
            priority_suggestion_expanded: None,
            priority_suggestion_area_trees: HashMap::new(),
            priority_suggestion_area_loading: None,
            priority_suggestion_area_error: None,
            priority_suggestion_area_filter: String::new(),
            priority_suggestion_area_loader: AsyncLoad::default(),
        }
    }

    pub(super) fn is_selected(&self, organization: &str, project: &str) -> bool {
        self.projects.iter().any(|entry| {
            entry.organization.eq_ignore_ascii_case(organization)
                && entry.project.eq_ignore_ascii_case(project)
        })
    }

    /// チェックボックスでの追加/削除。詳細パネルを開いていた行を外したら閉じる。
    pub(super) fn set_selected(&mut self, project_name: &str, selected: bool) {
        let organization = self.loaded_organization.clone();
        if selected {
            if !self.is_selected(&organization, project_name) {
                self.projects.push(AzureDevOpsProject {
                    organization: organization.clone(),
                    project: project_name.to_string(),
                    aliases: Vec::new(),
                    priority: 0,
                    include_pull_requests: true,
                    include_pipelines: true,
                    include_work_items: true,
                    interest_areas: Vec::new(),
                    interest_repositories: Vec::new(),
                });
            }
        } else {
            self.projects.retain(|entry| {
                !(entry.organization.eq_ignore_ascii_case(&organization)
                    && entry.project.eq_ignore_ascii_case(project_name))
            });
            if self.selected.as_ref().is_some_and(|(org, proj)| {
                org.eq_ignore_ascii_case(&organization) && proj == project_name
            }) {
                self.selected = None;
            }
        }
    }

    /// 一覧から行を選び、詳細パネルの編集バッファへ読み込む。
    pub(super) fn open_detail(&mut self, organization: &str, project: &str) {
        self.commit_text_fields();
        let Some((aliases_text, priority_text)) = self
            .find(organization, project)
            .map(|entry| (entry.aliases.join(", "), entry.priority.to_string()))
        else {
            return;
        };
        self.aliases_text = aliases_text;
        self.priority_text = priority_text;
        self.selected = Some((organization.to_string(), project.to_string()));
        self.area_nodes.clear();
        self.area_error = None;
        self.area_filter.clear();
        self.area_suggestions.clear();
        self.area_suggestion_error = None;
        self.start_area_load(organization.to_string(), project.to_string());
        self.repositories.clear();
        self.repository_error = None;
        self.repository_filter.clear();
        self.start_repository_load(organization.to_string(), project.to_string());
    }

    pub(super) fn find_project(
        &self,
        organization: &str,
        project: &str,
    ) -> Option<&AzureDevOpsProject> {
        self.find(organization, project)
    }

    fn find(&self, organization: &str, project: &str) -> Option<&AzureDevOpsProject> {
        self.projects.iter().find(|entry| {
            entry.organization.eq_ignore_ascii_case(organization)
                && entry.project.eq_ignore_ascii_case(project)
        })
    }

    fn find_mut(&mut self, organization: &str, project: &str) -> Option<&mut AzureDevOpsProject> {
        self.projects.iter_mut().find(|entry| {
            entry.organization.eq_ignore_ascii_case(organization)
                && entry.project.eq_ignore_ascii_case(project)
        })
    }

    /// テキスト欄 (aliases / priority) の内容を選択行へ書き戻す。
    /// 数値化できない優先度は 0 に丸める。行を切り替える直前・Apply 前に呼ぶ。
    pub(super) fn commit_text_fields(&mut self) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        let aliases_text = self.aliases_text.clone();
        let priority_text = self.priority_text.clone();
        if let Some(entry) = self.find_mut(&organization, &project) {
            entry.aliases = aliases_text
                .split(',')
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .map(str::to_string)
                .collect();
            entry.priority = priority_text.trim().parse().unwrap_or(0);
        }
    }

    /// チェックボックス 1 つ分のスコープ変更。
    pub(super) fn set_scope(
        &mut self,
        organization: &str,
        project: &str,
        scope: Scope,
        enabled: bool,
    ) {
        if let Some(entry) = self.find_mut(organization, project) {
            match scope {
                Scope::PullRequests => entry.include_pull_requests = enabled,
                Scope::Pipelines => entry.include_pipelines = enabled,
                Scope::WorkItems => entry.include_work_items = enabled,
            }
        }
    }

    pub(super) fn selected_interest_areas(&self) -> BTreeSet<String> {
        let Some((organization, project)) = &self.selected else {
            return BTreeSet::new();
        };
        self.find(organization, project)
            .map(|entry| entry.interest_areas.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn toggle_interest_area(&mut self, path: &str, enabled: bool) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.find_mut(&organization, &project) else {
            return;
        };
        let already = entry.interest_areas.iter().any(|area| area == path);
        if enabled && !already {
            entry.interest_areas.push(path.to_string());
        } else if !enabled && already {
            entry.interest_areas.retain(|area| area != path);
        }
    }

    pub(super) fn clear_interest_areas(&mut self) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        if let Some(entry) = self.find_mut(&organization, &project) {
            entry.interest_areas.clear();
        }
    }

    pub(super) fn selected_interest_repositories(&self) -> BTreeSet<String> {
        let Some((organization, project)) = &self.selected else {
            return BTreeSet::new();
        };
        self.find(organization, project)
            .map(|entry| entry.interest_repositories.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn toggle_interest_repository(&mut self, name: &str, enabled: bool) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        let Some(entry) = self.find_mut(&organization, &project) else {
            return;
        };
        let already = entry.interest_repositories.iter().any(|repo| repo == name);
        if enabled && !already {
            entry.interest_repositories.push(name.to_string());
        } else if !enabled && already {
            entry.interest_repositories.retain(|repo| repo != name);
        }
    }

    pub(super) fn clear_interest_repositories(&mut self) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        if let Some(entry) = self.find_mut(&organization, &project) {
            entry.interest_repositories.clear();
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
        let (sender, receiver) = channel();
        self.project_loader.start(receiver);
        self.loading = true;
        self.error = None;
        self.status = Some("Loading Azure DevOps projects...".to_string());
        thread::spawn(move || {
            let result = waypoint::azure_devops::list_projects(&organization, &pat);
            let _ = sender.send((organization, result));
        });
    }

    /// 非同期取得の完了を描画フレームで受け取る。
    pub(super) fn poll_load(&mut self) {
        self.poll_project_load();
        self.poll_area_load();
        self.poll_area_suggestion_load();
        self.poll_repository_load();
        self.poll_priority_suggestion_load();
        self.poll_priority_suggestion_area_load();
    }

    /// 進行中の非同期取得が 1 つでもあるか。
    ///
    /// **`poll_load` が見ている loader をすべて含めること。** 受信スレッドは
    /// egui のイベントループを起こせず、`poll_load` は描画のたびにしか
    /// 走らない。ここから漏れたローダは再描画が予約されず、結果が
    /// 永久に反映されない (提案モーダルの Area ツリーが実際にそうなった)。
    pub(super) fn is_loading_anything(&self) -> bool {
        self.loading
            || self.area_loading
            || self.area_suggestion_loading
            || self.repository_loading
            || self.priority_suggestion_loading
            || self.priority_suggestion_area_loading.is_some()
    }

    fn poll_project_load(&mut self) {
        let Some(result) = self.project_loader.poll() else {
            return;
        };
        self.loading = false;
        let Ok((organization, result)) = result else {
            self.status = None;
            self.error = Some("Azure DevOps project loading stopped unexpectedly.".to_string());
            return;
        };
        match result {
            Ok(projects) => {
                self.available_projects = projects;
                self.loaded_organization = organization;
                self.error = None;
                self.status = Some(format!(
                    "Loaded {} Azure DevOps projects.",
                    self.available_projects.len()
                ));
            }
            Err(error) => {
                self.status = None;
                self.error = Some(error);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    PullRequests,
    Pipelines,
    WorkItems,
}
