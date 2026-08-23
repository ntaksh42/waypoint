//! Azure DevOps プロジェクト選択画面の描画。

use eframe::egui;

use super::app::SettingsApp;
use super::keys::{dialog_keys, lock_modal_focus};
use super::trigger_draft::{merge_selected_azure_projects, parse_azure_projects};

impl SettingsApp {
    pub(super) fn show_azure_project_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = self.azure_project_picker.as_mut() else {
            return;
        };
        picker.poll_load();
        if picker.loading {
            // 受信スレッドは egui のイベントループを起こせないため、取得中だけ再描画する。
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Azure DevOps projects")
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 640.0])
            .min_size([500.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Organization");
                    ui.add(egui::TextEdit::singleline(&mut picker.organization).desired_width(240.0));
                });
                ui.horizontal(|ui| {
                    ui.label("PAT");
                    ui.add(
                        egui::TextEdit::singleline(&mut picker.pat)
                            .password(true)
                            .desired_width(300.0),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Save PAT and load projects").clicked() {
                        match waypoint::azure_devops::save_pat(&picker.organization, &picker.pat) {
                            Ok(()) => {
                                picker.pat.clear();
                                picker.start_load();
                            }
                            Err(error) => {
                                picker.status = None;
                                picker.error = Some(error);
                            }
                        }
                    }
                    if ui
                        .add_enabled(!picker.loading, egui::Button::new("Load projects"))
                        .clicked()
                    {
                        picker.start_load();
                    }
                    if ui.button("Delete PAT").clicked() {
                        match waypoint::azure_devops::delete_pat(&picker.organization) {
                            Ok(()) => {
                                picker.error = None;
                                picker.status = Some(
                                    "PAT removed from Windows Credential Manager.".to_string(),
                                );
                            }
                            Err(error) => {
                                picker.status = None;
                                picker.error = Some(error);
                            }
                        }
                    }
                });
                ui.weak("Required PAT scopes: Code (Read), Build (Read), Work Items (Read), Project and Team (Read).");
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.add(
                        egui::TextEdit::singleline(&mut picker.filter)
                            .hint_text("Filter by project name or alias")
                            .desired_width(320.0),
                    );
                    ui.checkbox(&mut picker.show_selected_only, "Selected only");
                    if picker.loading {
                        ui.spinner();
                        ui.label("Loading...");
                    }
                });

                let configured = parse_azure_projects(&picker.watched_projects).unwrap_or_default();
                let aliases_of = |project: &str| -> Vec<String> {
                    configured
                        .iter()
                        .find(|entry| {
                            entry.organization.eq_ignore_ascii_case(&picker.loaded_organization)
                                && entry.project.eq_ignore_ascii_case(project)
                        })
                        .map(|entry| entry.aliases.clone())
                        .unwrap_or_default()
                };

                let filter = picker.filter.to_lowercase();
                let mut filtered: Vec<_> = picker
                    .available_projects
                    .iter()
                    .filter(|project| picker.selected_projects.contains(*project) || !picker.show_selected_only)
                    .filter(|project| {
                        filter.is_empty()
                            || project.to_lowercase().contains(&filter)
                            || aliases_of(project)
                                .iter()
                                .any(|alias| alias.to_lowercase().contains(&filter))
                    })
                    .cloned()
                    .collect();
                // 選択済みを先頭に集め、大量の候補の中でも今の選択状態がすぐ見える。
                filtered.sort_by(|a, b| {
                    let a_selected = picker.selected_projects.contains(a);
                    let b_selected = picker.selected_projects.contains(b);
                    b_selected
                        .cmp(&a_selected)
                        .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
                });

                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} of {} projects shown; {} selected.",
                        filtered.len(),
                        picker.available_projects.len(),
                        picker.selected_projects.len()
                    ));
                    if ui.button("Select shown").clicked() {
                        picker.selected_projects.extend(filtered.iter().cloned());
                    }
                    if ui.button("Clear shown").clicked() {
                        for project in &filtered {
                            picker.selected_projects.remove(project);
                        }
                    }
                    if ui.button("Clear all").clicked() {
                        picker.selected_projects.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for project in &filtered {
                            let mut checked = picker.selected_projects.contains(project);
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut checked, project).changed() {
                                    if checked {
                                        picker.selected_projects.insert(project.clone());
                                    } else {
                                        picker.selected_projects.remove(project);
                                    }
                                }
                                let aliases = aliases_of(project);
                                if !aliases.is_empty() {
                                    ui.weak(format!("({})", aliases.join(", ")));
                                }
                            });
                        }
                    });
                ui.label("Advanced (aliases and priority; one project per line)");
                ui.add(
                    egui::TextEdit::multiline(&mut picker.watched_projects)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                ui.weak("Example: contoso/Waypoint | wp, launcher | 10 | pr,pipelines,wit");
                if let Some(status) = &picker.status {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, status);
                }
                if let Some(error) = &picker.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        let (_, dismiss) = dialog_keys(ctx, false);
        cancel |= dismiss;
        let applied_projects = if apply {
            match merge_selected_azure_projects(
                &picker.watched_projects,
                &picker.loaded_organization,
                &picker.selected_projects,
            ) {
                Ok(projects) => Some(projects),
                Err(error) => {
                    picker.error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        if let Some(projects) = applied_projects {
            if let Some(trigger) = self.trigger_draft.as_mut() {
                trigger.azure_projects = projects;
                trigger.error = None;
            }
            self.azure_project_picker = None;
        } else if cancel {
            self.azure_project_picker = None;
        }
    }
}
