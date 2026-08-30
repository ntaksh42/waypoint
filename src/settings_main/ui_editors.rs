//! 単一項目編集・変数編集・一括編集・移動先選択の各ダイアログ描画。

use eframe::egui;
use waypoint::config::OpenMode;

use super::app::SettingsApp;
use super::drafts::DraftKind;
use super::helpers::{filter_menu_choices, menu_choices, show_open_mode};
use super::keys::{dialog_keys, lock_modal_focus};

impl SettingsApp {
    pub(super) fn show_item_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new(if draft.editing.is_some() {
            "Edit item"
        } else {
            "Add item"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            match draft.kind {
                DraftKind::Folder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Path");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut draft.path);
                        if ui.button("Browse...").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            if draft.name.is_empty()
                                && let Some(name) = path.file_name()
                            {
                                draft.name = name.to_string_lossy().into_owned();
                            }
                            draft.path = path.display().to_string();
                        }
                    });
                    show_open_mode(ui, &mut draft.open);
                    ui.checkbox(&mut draft.show_branch, "Show Git branch name")
                        .on_hover_text(
                            "Appends [branch] to the menu label when the path is inside a Git work tree.",
                        );
                }
                DraftKind::File => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Path");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut draft.path);
                        if ui.button("Browse...").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_file()
                        {
                            if draft.name.is_empty()
                                && let Some(name) = path.file_name()
                            {
                                draft.name = name.to_string_lossy().into_owned();
                            }
                            draft.path = path.display().to_string();
                        }
                    });
                }
                DraftKind::SpecialFolder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Known folder");
                    egui::ComboBox::from_id_salt("known_folder")
                        .selected_text(&draft.known_folder)
                        .show_ui(ui, |ui| {
                            for name in waypoint::known_folder::NAMES {
                                ui.selectable_value(
                                    &mut draft.known_folder,
                                    (*name).to_string(),
                                    *name,
                                );
                            }
                        });
                    show_open_mode(ui, &mut draft.open);
                }
                DraftKind::Shell => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Target (e.g. shell:MyComputerFolder)");
                    ui.text_edit_singleline(&mut draft.target);
                }
                DraftKind::Submenu => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.checkbox(&mut draft.show_branch, "Show Git branch name for folders inside")
                        .on_hover_text(
                            "Applies showBranch to every Folder item nested under this menu, without editing them individually.",
                        );
                }
                DraftKind::Separator => {
                    ui.label("Heading (optional)");
                    ui.text_edit_singleline(&mut draft.name);
                }
            }

            if let Some(error) = &draft.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                apply = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        lock_modal_focus(ctx, &window);

        // 単一行の入力欄しか無いので Enter を確定に使える
        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            if let Some(error) = draft.validate() {
                draft.error = Some(error);
            } else {
                self.apply_draft();
            }
        } else if cancel || dismiss {
            self.draft = None;
        }
    }

    pub(super) fn show_variables_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.variables_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let mut remove = None;
        let window = egui::Window::new("Variables")
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("variables_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Value");
                        ui.end_row();
                        for (index, (name, value)) in draft.entries.iter_mut().enumerate() {
                            ui.text_edit_singleline(name);
                            ui.text_edit_singleline(value);
                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if ui.button("Add variable").clicked() {
                    draft.entries.push((String::new(), String::new()));
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Save").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        if let Some(index) = remove {
            draft.entries.remove(index);
        }
        // 名前と値はどちらも単一行なので Enter を確定に使える
        let (accept, dismiss) = dialog_keys(ctx, true);
        let cancel = cancel || dismiss;
        if apply || accept {
            let mut variables = std::collections::BTreeMap::new();
            let mut error = None;
            for (name, value) in &draft.entries {
                let name = name.trim();
                if name.is_empty() {
                    error = Some("Variable name is required.".to_string());
                    break;
                }
                if variables.insert(name.to_string(), value.clone()).is_some() {
                    error = Some(format!("Variable name is duplicated: {name}"));
                    break;
                }
            }
            if let Some(error) = error {
                draft.error = Some(error);
            } else {
                self.config.variables = variables;
                self.variables_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.variables_draft = None;
        }
    }

    /// 選択中の複数項目に共通するプロパティをまとめて設定するダイアログ (FR-6) 。
    /// 各欄は 3 状態 (変更しない / 値A / 値B) で、既定は「変更しない」。
    pub(super) fn show_batch_editor(&mut self, ctx: &egui::Context) {
        let selected_count = self.selected_items.len();
        let Some(draft) = self.batch_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Edit selected items")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Applies to {selected_count} (Folder / Special folder only where relevant)."
                ));
                ui.separator();
                ui.label("Open mode");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut draft.open, None, "Don't change");
                    ui.selectable_value(&mut draft.open, Some(OpenMode::NewWindow), "New window");
                    ui.selectable_value(
                        &mut draft.open,
                        Some(OpenMode::Reuse),
                        "Reuse Explorer window",
                    );
                });
                ui.label("Show Git branch name");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut draft.show_branch, None, "Don't change");
                    ui.selectable_value(&mut draft.show_branch, Some(true), "On");
                    ui.selectable_value(&mut draft.show_branch, Some(false), "Off");
                });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            self.apply_batch_draft();
        } else if cancel || dismiss {
            self.batch_draft = None;
        }
    }

    /// 選択中の項目を別メニューへまとめて移すダイアログ (FR-6.3) 。
    pub(super) fn show_move_to_menu_editor(&mut self, ctx: &egui::Context) {
        let prompt = format!("Move {} to:", self.selection_summary().to_ascii_lowercase());
        let Some(draft) = self.move_to_menu_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let choices = menu_choices(&self.config);
        let selected_name = choices
            .iter()
            .find(|(path, _)| path == &draft.target)
            .map(|(_, name)| name.as_str())
            .unwrap_or("Main");
        let window = egui::Window::new("Move to menu")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&prompt);
                egui::ComboBox::from_id_salt("move_to_menu_target")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.filter)
                                .hint_text("Filter menus")
                                .desired_width(f32::INFINITY),
                        );
                        ui.separator();
                        for (path, name) in filter_menu_choices(&choices, &draft.filter) {
                            ui.selectable_value(&mut draft.target, path.clone(), name);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Move").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            let target = draft.target.clone();
            self.move_selected_to_menu(&target);
            self.move_to_menu_draft = None;
        } else if cancel || dismiss {
            self.move_to_menu_draft = None;
        }
    }
}
