//! 詳細パネル (`AzureProjectPicker::open_detail`) が選択行を読み込む際の
//! 非同期読み込み: Area Path 階層・リポジトリ名一覧・自分の Area 提案。

use std::sync::mpsc::channel;
use std::thread;

use super::AzureProjectPicker;

impl AzureProjectPicker {
    pub(super) fn start_area_load(&mut self, organization: String, project: String) {
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.area_loader.start(receiver);
        self.area_loading = true;
        self.area_error = None;
        thread::spawn(move || {
            let result = waypoint::azure_devops::fetch_area_nodes(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    pub(super) fn poll_area_load(&mut self) {
        let Some(result) = self.area_loader.poll() else {
            return;
        };
        self.area_loading = false;
        let Ok((key, result)) = result else {
            self.area_error = Some("Area Path loading stopped unexpectedly.".to_string());
            return;
        };
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

    pub(super) fn start_repository_load(&mut self, organization: String, project: String) {
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.repository_loader.start(receiver);
        self.repository_loading = true;
        self.repository_error = None;
        thread::spawn(move || {
            let result =
                waypoint::azure_devops::list_repository_names(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    pub(super) fn poll_repository_load(&mut self) {
        let Some(result) = self.repository_loader.poll() else {
            return;
        };
        self.repository_loading = false;
        let Ok((key, result)) = result else {
            self.repository_error = Some("Repository loading stopped unexpectedly.".to_string());
            return;
        };
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
    pub(crate) fn suggest_areas_from_my_work_items(&mut self) {
        let Some((organization, project)) = self.selected.clone() else {
            return;
        };
        let pat = self.pat.clone();
        let (sender, receiver) = channel();
        self.area_suggestion_loader.start(receiver);
        self.area_suggestion_loading = true;
        self.area_suggestion_error = None;
        thread::spawn(move || {
            let result =
                waypoint::azure_devops::fetch_my_area_suggestions(&organization, &project, &pat);
            let _ = sender.send(((organization, project), result));
        });
    }

    pub(super) fn poll_area_suggestion_load(&mut self) {
        let Some(result) = self.area_suggestion_loader.poll() else {
            return;
        };
        self.area_suggestion_loading = false;
        let Ok((key, result)) = result else {
            self.area_suggestion_error =
                Some("Area Path suggestion loading stopped unexpectedly.".to_string());
            return;
        };
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
}
