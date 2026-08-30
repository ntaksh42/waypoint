//! 優先度提案モーダル: 監視中の全プロジェクトを横断し、直近アクティビティ
//! (アサイン + メンション) から Project / Area の優先度を提案する。
//!
//! 行の選択 (`AzureProjectPicker::selected`) とは独立したモーダル状態を
//! 持つため、`AzureProjectPicker` へのメソッド実装だけをこのファイルへ
//! 分離してある (フィールド自体は `mod.rs` の struct 定義に残る)。

use std::sync::mpsc::channel;
use std::thread;

use waypoint::config::AzureDevOpsSettings;

use super::AzureProjectPicker;

impl AzureProjectPicker {
    /// 監視中の全プロジェクトを横断し、直近 90 日のアサイン・メンション件数から
    /// Project / Area の優先度を提案する。結果はフィルタ可能な専用モーダルで表示する。
    pub(crate) fn start_priority_suggestions(&mut self) {
        self.commit_text_fields();
        let settings = AzureDevOpsSettings {
            enabled: self.enabled,
            projects: self.projects.clone(),
        };
        let receiver = waypoint::azure_devops::suggest_priorities_async(settings);
        self.priority_suggestion_loader.start(receiver);
        self.priority_suggestion_loading = true;
        self.priority_suggestion_error = None;
        self.priority_suggestion_open = true;
        self.priority_suggestion_filter.clear();
        self.priority_suggestion_area_filter.clear();
        self.priority_suggestion_checked.clear();
    }

    pub(crate) fn poll_priority_suggestion_load(&mut self) {
        let Some(result) = self.priority_suggestion_loader.poll() else {
            return;
        };
        self.priority_suggestion_loading = false;
        let Ok(result) = result else {
            self.priority_suggestion_error =
                Some("Activity loading stopped unexpectedly.".to_string());
            return;
        };
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
    pub(crate) fn apply_priority_suggestions(&mut self) {
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
    pub(crate) fn toggle_priority_suggestion_expanded(
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
        self.priority_suggestion_area_loader.start(receiver);
        self.priority_suggestion_area_loading = Some(key.clone());
        self.priority_suggestion_area_error = None;
        thread::spawn(move || {
            let result = waypoint::azure_devops::fetch_area_nodes(&key.0, &key.1, &pat);
            let _ = sender.send((key, result));
        });
    }

    pub(crate) fn poll_priority_suggestion_area_load(&mut self) {
        let Some(result) = self.priority_suggestion_area_loader.poll() else {
            return;
        };
        self.priority_suggestion_area_loading = None;
        let Ok((key, result)) = result else {
            self.priority_suggestion_area_error =
                Some("Area Path loading stopped unexpectedly.".to_string());
            return;
        };
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
    pub(crate) fn check_recent_activity_areas(&mut self, key: &(String, String)) {
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
    pub(crate) fn toggle_priority_suggestion_area(
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
}
