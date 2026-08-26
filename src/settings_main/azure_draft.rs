//! Azure DevOps プロジェクト選択画面の状態とロジック。
//!
//! `projects` に設定済みプロジェクトをそのまま保持し、一覧から選んだ行を
//! `selected` (organization, project) で指し示す。詳細パネルの編集は
//! `projects` の該当エントリへ都度書き戻すので、Advanced テキスト DSL の
//! ような別表現との相互変換は発生しない。

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;

use waypoint::azure_devops::{AreaNode, ProjectActivity};
use waypoint::config::{AzureDevOpsProject, AzureDevOpsSettings, Config};

type ProjectListLoad = (String, Result<Vec<String>, String>);
type AreaLoad = ((String, String), Result<Vec<AreaNode>, String>);
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
    project_loader: Option<Receiver<ProjectListLoad>>,
    pub(super) loading: bool,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
    /// 詳細パネルで開いている行 (organization, project)。
    pub(super) selected: Option<(String, String)>,
    /// 選択行の Area Path 階層。行を切り替えるたびに読み直す。
    pub(super) area_nodes: Vec<AreaNode>,
    area_loader: Option<Receiver<AreaLoad>>,
    pub(super) area_loading: bool,
    pub(super) area_error: Option<String>,
    pub(super) area_filter: String,
    /// 自分に割り当てられた Work Item から集計した Area Path 候補 (パス, 件数)。
    /// 明示的にボタンを押した時だけ取得する。
    pub(super) area_suggestions: Vec<(String, usize)>,
    area_suggestion_loader: Option<Receiver<AreaSuggestionLoad>>,
    pub(super) area_suggestion_loading: bool,
    pub(super) area_suggestion_error: Option<String>,
    /// 選択行のリポジトリ名一覧。行を切り替えるたびに読み直す。
    pub(super) repositories: Vec<String>,
    repository_loader: Option<Receiver<RepositoryLoad>>,
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
    priority_suggestion_loader: Option<Receiver<Result<Vec<ProjectActivity>, String>>>,
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
    priority_suggestion_area_loader: Option<Receiver<AreaLoad>>,
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
            project_loader: None,
            loading: false,
            status: None,
            error: None,
            selected: None,
            area_nodes: Vec::new(),
            area_loader: None,
            area_loading: false,
            area_error: None,
            area_filter: String::new(),
            area_suggestions: Vec::new(),
            area_suggestion_loader: None,
            area_suggestion_loading: false,
            area_suggestion_error: None,
            repositories: Vec::new(),
            repository_loader: None,
            repository_loading: false,
            repository_error: None,
            repository_filter: String::new(),
            aliases_text: String::new(),
            priority_text: String::new(),
            priority_suggestions: Vec::new(),
            priority_suggestion_loader: None,
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
            priority_suggestion_area_loader: None,
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
        self.project_loader = Some(receiver);
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
        let Some(receiver) = self.project_loader.as_ref() else {
            return;
        };
        let (organization, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.project_loader = None;
                self.loading = false;
                self.status = None;
                self.error = Some("Azure DevOps project loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.project_loader = None;
        self.loading = false;
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

    fn start_area_load(&mut self, organization: String, project: String) {
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.area_loader = Some(receiver);
        self.area_loading = true;
        self.area_error = None;
        thread::spawn(move || {
            let result = waypoint::azure_devops::fetch_area_nodes(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    fn poll_area_load(&mut self) {
        let Some(receiver) = self.area_loader.as_ref() else {
            return;
        };
        let (key, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.area_loader = None;
                self.area_loading = false;
                self.area_error = Some("Area Path loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.area_loader = None;
        self.area_loading = false;
        // 取得中に別の行へ切り替えていたら、古い結果は捨てる。
        if self.selected.as_ref() != Some(&key) {
            return;
        }
        match result {
            Ok(nodes) => {
                self.area_nodes = nodes;
                self.area_error = None;
            }
            Err(error) => self.area_error = Some(error),
        }
    }

    fn start_repository_load(&mut self, organization: String, project: String) {
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.repository_loader = Some(receiver);
        self.repository_loading = true;
        self.repository_error = None;
        thread::spawn(move || {
            let result =
                waypoint::azure_devops::list_repository_names(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    fn poll_repository_load(&mut self) {
        let Some(receiver) = self.repository_loader.as_ref() else {
            return;
        };
        let (key, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.repository_loader = None;
                self.repository_loading = false;
                self.repository_error =
                    Some("Repository loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.repository_loader = None;
        self.repository_loading = false;
        // 取得中に別の行へ切り替えていたら、古い結果は捨てる。
        if self.selected.as_ref() != Some(&key) {
            return;
        }
        match result {
            Ok(names) => {
                self.repositories = names;
                self.repository_error = None;
            }
            Err(error) => self.repository_error = Some(error),
        }
    }

    /// 選択中プロジェクトで、自分に割り当てられた Work Item から
    /// 興味のある Area Path の候補を集計する。
    pub(super) fn suggest_areas_from_my_work_items(&mut self) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.area_suggestion_loader = Some(receiver);
        self.area_suggestion_loading = true;
        self.area_suggestion_error = None;
        thread::spawn(move || {
            let result =
                waypoint::azure_devops::fetch_my_area_suggestions(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    fn poll_area_suggestion_load(&mut self) {
        let Some(receiver) = self.area_suggestion_loader.as_ref() else {
            return;
        };
        let (key, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.area_suggestion_loader = None;
                self.area_suggestion_loading = false;
                self.area_suggestion_error =
                    Some("Area Path suggestion loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.area_suggestion_loader = None;
        self.area_suggestion_loading = false;
        // 取得中に別の行へ切り替えていたら、古い結果は捨てる。
        if self.selected.as_ref() != Some(&key) {
            return;
        }
        match result {
            Ok(suggestions) => {
                self.area_suggestions = suggestions;
                self.area_suggestion_error = None;
                if self.area_suggestions.is_empty() {
                    self.area_suggestion_error =
                        Some("No work items are assigned to you in this project.".to_string());
                }
            }
            Err(error) => self.area_suggestion_error = Some(error),
        }
    }

    /// 監視中の全プロジェクトを横断し、直近 90 日のアサイン・メンション件数から
    /// Project / Area の優先度を提案する。結果はフィルタ可能な専用モーダルで表示する。
    pub(super) fn start_priority_suggestions(&mut self) {
        self.commit_text_fields();
        let settings = AzureDevOpsSettings {
            enabled: self.enabled,
            projects: self.projects.clone(),
        };
        let receiver = waypoint::azure_devops::suggest_priorities_async(settings);
        self.priority_suggestion_loader = Some(receiver);
        self.priority_suggestion_loading = true;
        self.priority_suggestion_error = None;
        self.priority_suggestion_open = true;
        self.priority_suggestion_filter.clear();
        self.priority_suggestion_area_filter.clear();
        self.priority_suggestion_checked.clear();
    }

    fn poll_priority_suggestion_load(&mut self) {
        let Some(receiver) = self.priority_suggestion_loader.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.priority_suggestion_loader = None;
                self.priority_suggestion_loading = false;
                self.priority_suggestion_error =
                    Some("Activity loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.priority_suggestion_loader = None;
        self.priority_suggestion_loading = false;
        match result {
            Ok(mut activity) => {
                activity.sort_by(|a, b| {
                    b.count
                        .cmp(&a.count)
                        .then_with(|| a.project.to_lowercase().cmp(&b.project.to_lowercase()))
                });
                if activity.iter().all(|entry| entry.count == 0) {
                    self.priority_suggestion_error = Some(
                        "No recent assignments or mentions found in the last 90 days.".to_string(),
                    );
                }
                self.priority_suggestions = activity;
            }
            Err(error) => self.priority_suggestion_error = Some(error),
        }
    }

    /// チェックした提案を確定する。Project は活動件数の多い順に `priority`
    /// (0, 1, 2, ...) を振り直し、各プロジェクトの Area 提案は上位 1 件を
    /// `interest_areas` へ追加する (すでに含まれていれば何もしない)。
    /// チェックしなかったプロジェクトの優先度はそのまま変更しない。Area は
    /// このメソッドでは触らない (ツリーを展開してチェックした時点で
    /// `toggle_priority_suggestion_area` が都度 `interest_areas` へ反映済み)。
    pub(super) fn apply_priority_suggestions(&mut self) {
        let checked: Vec<_> = self
            .priority_suggestions
            .iter()
            .filter(|entry| {
                self.priority_suggestion_checked
                    .contains(&(entry.organization.clone(), entry.project.clone()))
            })
            .cloned()
            .collect();
        for (index, entry) in checked.iter().enumerate() {
            if let Some(project) = self.find_mut(&entry.organization, &entry.project) {
                project.priority = index as u32;
            }
        }
        self.priority_suggestion_open = false;
        self.status = Some(format!(
            "Applied priority from recent activity to {} project(s).",
            checked.len()
        ));
    }

    /// 提案モーダルでプロジェクト行のツリー展開をトグルする。展開時、
    /// まだ取得していなければ Area ツリーを非同期で取り込む
    /// (一度取得したプロジェクトはキャッシュを再利用し、開閉のたびに叩かない)。
    pub(super) fn toggle_priority_suggestion_expanded(
        &mut self,
        organization: &str,
        project: &str,
    ) {
        let key = (organization.to_string(), project.to_string());
        // 別ツリーへ移るので絞り込みは持ち越さない。残すと、開いた直後に
        // 前の Area 名で 0 件になり「ツリーが空」に見える。
        self.priority_suggestion_area_filter.clear();
        if self.priority_suggestion_expanded.as_ref() == Some(&key) {
            self.priority_suggestion_expanded = None;
            return;
        }
        self.priority_suggestion_expanded = Some(key.clone());
        if self.priority_suggestion_area_trees.contains_key(&key) {
            return;
        }
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.priority_suggestion_area_loader = Some(receiver);
        self.priority_suggestion_area_loading = Some(key.clone());
        self.priority_suggestion_area_error = None;
        thread::spawn(move || {
            let result = waypoint::azure_devops::fetch_area_nodes(&key.0, &key.1, &pat);
            let _ = sender.send((key, result));
        });
    }

    fn poll_priority_suggestion_area_load(&mut self) {
        let Some(receiver) = self.priority_suggestion_area_loader.as_ref() else {
            return;
        };
        let (key, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.priority_suggestion_area_loader = None;
                self.priority_suggestion_area_loading = None;
                self.priority_suggestion_area_error =
                    Some("Area Path loading stopped unexpectedly.".to_string());
                return;
            }
        };
        self.priority_suggestion_area_loader = None;
        self.priority_suggestion_area_loading = None;
        match result {
            Ok(nodes) => {
                self.priority_suggestion_area_trees
                    .insert(key.clone(), nodes);
                self.priority_suggestion_area_error = None;
                self.check_recent_activity_areas(&key);
            }
            Err(error) => self.priority_suggestion_area_error = Some(error),
        }
    }

    /// Area ツリーを初めて読み込んだ直後、そのプロジェクトの直近アクティビティ
    /// (アサイン + メンション) に出現した Area Path をデフォルトでチェック済みに
    /// する (`interest_areas` へ追加する)。ツリーはプロジェクトごとに一度しか
    /// 取得しないため、ユーザーが手動でチェックを外してもツリーを開き直した
    /// だけでは再度チェックされ直さない。
    pub(super) fn check_recent_activity_areas(&mut self, key: &(String, String)) {
        let paths: Vec<String> = self
            .priority_suggestions
            .iter()
            .find(|entry| {
                entry.organization.eq_ignore_ascii_case(&key.0)
                    && entry.project.eq_ignore_ascii_case(&key.1)
            })
            .map(|entry| entry.areas.iter().map(|(path, _)| path.clone()).collect())
            .unwrap_or_default();
        let Some(project_entry) = self.find_mut(&key.0, &key.1) else {
            return;
        };
        for path in paths {
            if !project_entry
                .interest_areas
                .iter()
                .any(|area| area == &path)
            {
                project_entry.interest_areas.push(path);
            }
        }
    }

    /// 展開中プロジェクトの Area チェックボックスをトグルし、即座に
    /// そのプロジェクトの `interest_areas` へ反映する (提案モーダルは
    /// Project の priority と違い、Apply を待たず即時に効かせる — Area は
    /// 個別プロジェクトの設定そのものであり、他プロジェクトの選択と
    /// 競合しないため)。
    pub(super) fn toggle_priority_suggestion_area(
        &mut self,
        organization: &str,
        project: &str,
        path: &str,
        enabled: bool,
    ) {
        let Some(entry) = self.find_mut(organization, project) else {
            return;
        };
        let already = entry.interest_areas.iter().any(|area| area == path);
        if enabled && !already {
            entry.interest_areas.push(path.to_string());
        } else if !enabled && already {
            entry.interest_areas.retain(|area| area != path);
        }
    }

    /// 監視プロジェクト一覧を JSON ファイルへ書き出す。PAT は含めない
    /// (`AzureDevOpsProject` に PAT フィールド自体が無い) ので、そのまま
    /// 共有・バックアップしてよい。
    pub(super) fn export_to_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("azure-devops-projects.json")
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        match encode_projects_json(&self.projects) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => {
                    self.error = None;
                    self.status = Some(format!(
                        "Exported {} project(s) to {}.",
                        self.projects.len(),
                        path.display()
                    ));
                }
                Err(error) => {
                    self.status = None;
                    self.error = Some(format!("Could not write file: {error}"));
                }
            },
            Err(error) => {
                self.status = None;
                self.error = Some(format!("Could not encode projects: {error}"));
            }
        }
    }

    /// JSON ファイルから監視プロジェクト一覧を読み込み、現在の一覧を丸ごと置き換える。
    /// 開いていた詳細パネルは選択解除する (置き換え後は別のエントリを指すため)。
    pub(super) fn import_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|error| format!("Could not read file: {error}"))
            .and_then(|contents| decode_projects_json(&contents));
        match result {
            Ok(projects) => {
                let count = projects.len();
                self.projects = projects;
                self.selected = None;
                self.error = None;
                self.status = Some(format!(
                    "Imported {count} project(s) from {}.",
                    path.display()
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

pub(super) fn encode_projects_json(projects: &[AzureDevOpsProject]) -> Result<String, String> {
    serde_json::to_string_pretty(projects).map_err(|error| error.to_string())
}

pub(super) fn decode_projects_json(contents: &str) -> Result<Vec<AzureDevOpsProject>, String> {
    serde_json::from_str(contents).map_err(|error| format!("Could not parse file: {error}"))
}
