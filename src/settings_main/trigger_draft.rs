//! トリガー設定ダイアログの状態 (ホットキー・Quick Launch 全般)。
//! Azure DevOps プロジェクト選択の状態とロジックは `azure_draft` に分離する。

use waypoint::config::Config;
use waypoint::hotkey_capture;

/// トリガー設定ダイアログのタブ。設定項目が多いため用途別に分ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TriggerTab {
    General,
    QuickLaunch,
    AzureDevOps,
}

pub(super) struct TriggerDraft {
    pub(super) active_tab: TriggerTab,
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
    pub(super) azure_projects: Vec<waypoint::config::AzureDevOpsProject>,
    pub(super) include_everything: bool,
    pub(super) search_paths: bool,
    pub(super) visible_results: usize,
    pub(super) error: Option<String>,
    /// キー入力から記録中の欄 (FR-6.8.1) 。
    pub(super) recording: Option<HotkeyField>,
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
            active_tab: TriggerTab::General,
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
            azure_projects: quick_launch.azure_devops.projects.clone(),
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

pub(super) fn azure_project_count(draft: &TriggerDraft) -> usize {
    draft.azure_projects.len()
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
