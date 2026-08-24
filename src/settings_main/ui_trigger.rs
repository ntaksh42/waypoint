//! トリガー・Quick Launch 設定ダイアログの描画。

use eframe::egui;
use waypoint::hotkey_capture;

use super::app::SettingsApp;
use super::keys::{dialog_keys, lock_modal_focus};
use super::trigger_draft::{HotkeyField, TriggerTab, hotkey_row, poll_hotkey_capture};

impl SettingsApp {
    pub(super) fn show_trigger_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.trigger_draft.as_mut() else {
            return;
        };
        poll_hotkey_capture(ctx, draft);
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Trigger")
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .default_height(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut draft.active_tab, TriggerTab::General, "General");
                    ui.selectable_value(
                        &mut draft.active_tab,
                        TriggerTab::QuickLaunch,
                        "Quick Launch",
                    );
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| match draft.active_tab {
                    TriggerTab::General => {
                        ui.checkbox(&mut draft.middle_click, "Enable middle click");
                        ui.label("Hotkey");
                        hotkey_row(ui, draft, HotkeyField::Menu);
                        ui.weak("Example: Win+W");
                        ui.add_space(8.0);
                        ui.label("Excluded processes (one per line)");
                        ui.add(
                            egui::TextEdit::multiline(&mut draft.excluded_processes)
                                .desired_rows(7)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    TriggerTab::QuickLaunch => {
                        ui.label("Hotkey");
                        hotkey_row(ui, draft, HotkeyField::QuickLaunch);
                        ui.add_space(8.0);
                        ui.checkbox(&mut draft.include_recent_folders, "Include Recent Folders");
                        ui.checkbox(
                            &mut draft.include_frequent_folders,
                            "Include Frequent Folders",
                        );
                        ui.checkbox(&mut draft.include_open_windows, "Include Open Windows");
                        ui.checkbox(
                            &mut draft.include_bookmarks,
                            "Include browser bookmarks (type \"b \" to search)",
                        );
                        ui.checkbox(
                            &mut draft.include_browser_history,
                            "Include browser history (type \"h \" to search)",
                        );
                        ui.checkbox(
                            &mut draft.include_apps,
                            "Include installed apps (type \"a \" to search)",
                        );
                        ui.checkbox(
                            &mut draft.include_everything,
                            "Search all files via Everything (type \"f \" to search; requires Everything to be running)",
                        );
                        ui.checkbox(&mut draft.search_paths, "Search folder paths");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Visible results");
                            ui.add(egui::DragValue::new(&mut draft.visible_results).range(12..=24));
                        });
                    }
                });
                ui.separator();
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        // 除外プロセス欄が複数行なので Enter は改行に譲り、Esc だけ受ける。
        // 記録中の打鍵はフックが握り潰すため、ここには Esc は届かない
        let (_, dismiss) = dialog_keys(ctx, false);
        let cancel = cancel || dismiss;
        if apply || cancel {
            // 画面を閉じるならフックを残さない
            hotkey_capture::stop();
            draft.recording = None;
        }
        if apply {
            if waypoint::trigger::parse_hotkey(draft.hotkey.trim()).is_none() {
                draft.error = Some("Hotkey is invalid.".to_string());
            } else if waypoint::trigger::parse_hotkey(draft.quick_launch_hotkey.trim()).is_none() {
                draft.error = Some("Quick Launch hotkey is invalid.".to_string());
            } else if draft
                .hotkey
                .trim()
                .eq_ignore_ascii_case(draft.quick_launch_hotkey.trim())
            {
                draft.error = Some("The two hotkeys must be different.".to_string());
            } else {
                self.config.settings.trigger.middle_click = draft.middle_click;
                self.config.settings.trigger.hotkey = draft.hotkey.trim().to_string();
                self.config.settings.trigger.excluded_processes = draft
                    .excluded_processes
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                self.config.settings.quick_launch.hotkey =
                    draft.quick_launch_hotkey.trim().to_string();
                self.config.settings.quick_launch.include_recent_folders =
                    draft.include_recent_folders;
                self.config.settings.quick_launch.include_frequent_folders =
                    draft.include_frequent_folders;
                self.config.settings.quick_launch.include_open_windows = draft.include_open_windows;
                self.config.settings.quick_launch.include_bookmarks = draft.include_bookmarks;
                self.config.settings.quick_launch.include_browser_history =
                    draft.include_browser_history;
                self.config.settings.quick_launch.include_apps = draft.include_apps;
                self.config.settings.quick_launch.include_everything = draft.include_everything;
                self.config.settings.quick_launch.search_paths = draft.search_paths;
                self.config.settings.quick_launch.visible_results =
                    draft.visible_results.clamp(12, 24);
                self.dirty = true;
                self.status = None;
                if self.save() {
                    self.trigger_draft = None;
                }
            }
        } else if cancel {
            self.trigger_draft = None;
        }
    }
}
