//! Azure DevOps 設定画面の描画。
//!
//! 有効化トグルと監視プロジェクトの選択・詳細編集を 1 つのウィンドウに
//! まとめる。上部に有効化チェックボックスとキャッシュ鮮度、その下に
//! 左は検索・選択可能なプロジェクト一覧、右は選択中プロジェクトの詳細編集
//! (エイリアス・優先度・同期スコープ・興味のある Area Path) を並べる。
//! 大規模な組織でも一覧はフィルタで絞り込め、詳細は 1 プロジェクトずつ
//! 個別に設定できる。OK/Cancel は 1 組だけで、有効化トグルとプロジェクト
//! 選択の両方を一度にコミットする (以前は 2 段のダイアログにそれぞれ
//! 独立した OK/Apply があり、片方だけ保存し忘れる罠があった)。

use eframe::egui;
use waypoint::config::AzureDevOpsSettings;

use super::app::SettingsApp;
use super::azure_draft::{AzureProjectPicker, Scope};
use super::keys::{dialog_keys, lock_modal_focus};

impl SettingsApp {
    pub(super) fn show_azure_project_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = self.azure_project_picker.as_mut() else {
            return;
        };
        picker.poll_load();
        if picker.loading || picker.area_loading {
            // 受信スレッドは egui のイベントループを起こせないため、取得中だけ再描画する。
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Azure DevOps")
            .collapsible(false)
            .resizable(true)
            .default_size([1000.0, 860.0])
            .min_size([760.0, 520.0])
            .max_height(
                ctx.input(|input| input.viewport().outer_rect)
                    .map_or(900.0, |rect| rect.height() * 0.92),
            )
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.checkbox(
                    &mut picker.enabled,
                    "Enable Azure DevOps search (type \"az \" to search)",
                );
                let azure_status = waypoint::azure_devops::cache_status(&AzureDevOpsSettings {
                    enabled: picker.enabled,
                    projects: picker.projects.clone(),
                });
                ui.weak(waypoint::azure_devops::cache_status_label(&azure_status));
                if let Some(error) = azure_status.last_error {
                    ui.weak(format!("Last Azure DevOps error: {error}"));
                }
                ui.separator();
                show_connection_row(ui, picker);
                ui.separator();
                // OK/Cancel とステータス行の高さを確保してから、残りを一覧・詳細に回す。
                let footer_height = 76.0;
                let body_height = (ui.available_height() - footer_height).max(240.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(body_height);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() * 0.38);
                        show_project_list(ui, picker, body_height);
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("azure_detail_panel")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                show_detail_panel(ui, picker);
                            });
                    });
                });
                ui.separator();
                if let Some(status) = &picker.status {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, status);
                }
                if let Some(error) = &picker.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Export...")
                        .on_hover_text("Save the watched project list to a JSON file")
                        .clicked()
                    {
                        picker.export_to_file();
                    }
                    if ui
                        .button("Import...")
                        .on_hover_text(
                            "Replace the watched project list with one loaded from a JSON file",
                        )
                        .clicked()
                    {
                        picker.import_from_file();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = ui.button("Cancel").clicked();
                        apply = ui.button("OK").clicked();
                    });
                });
            });
        lock_modal_focus(ctx, &window);

        let (_, dismiss) = dialog_keys(ctx, false);
        cancel |= dismiss;
        if apply {
            picker.commit_text_fields();
            self.config.settings.quick_launch.azure_devops = AzureDevOpsSettings {
                enabled: picker.enabled,
                projects: picker.projects.clone(),
            };
            self.dirty = true;
            self.status = None;
            if self.save() {
                self.azure_project_picker = None;
            }
        } else if cancel {
            self.azure_project_picker = None;
        }
    }
}

fn show_connection_row(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    ui.horizontal(|ui| {
        ui.label("Organization");
        ui.add(egui::TextEdit::singleline(&mut picker.organization).desired_width(200.0));
        ui.label("PAT");
        ui.add(
            egui::TextEdit::singleline(&mut picker.pat)
                .password(true)
                .desired_width(220.0),
        );
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
                    picker.status =
                        Some("PAT removed from Windows Credential Manager.".to_string());
                }
                Err(error) => {
                    picker.status = None;
                    picker.error = Some(error);
                }
            }
        }
        if picker.loading {
            ui.spinner();
            ui.label("Loading...");
        }
    });
    ui.weak("Required PAT scopes: Code (Read), Build (Read), Work Items (Read), Project and Team (Read).");
}

/// 左ペイン: フィルタ・一括操作・チェックボックス一覧。
fn show_project_list(ui: &mut egui::Ui, picker: &mut AzureProjectPicker, available_height: f32) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut picker.filter)
                .hint_text("Filter by project name or alias")
                .desired_width(ui.available_width().min(220.0)),
        );
    });
    ui.checkbox(&mut picker.show_selected_only, "Selected only");

    let organization = picker.loaded_organization.clone();
    let aliases_by_project: std::collections::HashMap<String, Vec<String>> = picker
        .projects
        .iter()
        .filter(|entry| entry.organization.eq_ignore_ascii_case(&organization))
        .map(|entry| (entry.project.to_lowercase(), entry.aliases.clone()))
        .collect();
    let aliases_of = |project: &str| -> Vec<String> {
        aliases_by_project
            .get(&project.to_lowercase())
            .cloned()
            .unwrap_or_default()
    };

    let filter = picker.filter.to_lowercase();
    let mut filtered: Vec<_> = picker
        .available_projects
        .iter()
        .filter(|project| picker.is_selected(&organization, project) || !picker.show_selected_only)
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
        let a_selected = picker.is_selected(&organization, a);
        let b_selected = picker.is_selected(&organization, b);
        b_selected
            .cmp(&a_selected)
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });

    ui.horizontal(|ui| {
        ui.label(format!(
            "{} of {} shown; {} selected",
            filtered.len(),
            picker.available_projects.len(),
            picker
                .projects
                .iter()
                .filter(|entry| entry.organization.eq_ignore_ascii_case(&organization))
                .count()
        ));
    });
    ui.horizontal(|ui| {
        if ui.button("Select shown").clicked() {
            for project in &filtered {
                picker.set_selected(project, true);
            }
        }
        if ui.button("Clear shown").clicked() {
            for project in &filtered {
                picker.set_selected(project, false);
            }
        }
        if ui.button("Clear all").clicked() {
            for project in picker.available_projects.clone() {
                picker.set_selected(&project, false);
            }
        }
    });

    let list_height = (available_height - ui.min_rect().height()).max(160.0);
    egui::ScrollArea::vertical()
        .id_salt("azure_project_list")
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for project in &filtered {
                let mut checked = picker.is_selected(&organization, project);
                let is_open = picker.selected.as_ref().is_some_and(|(org, proj)| {
                    org.eq_ignore_ascii_case(&organization) && proj == project
                });
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut checked, "").changed() {
                        picker.set_selected(project, checked);
                    }
                    let label = ui.selectable_label(is_open, project);
                    if label.clicked() && checked {
                        picker.open_detail(&organization, project);
                    }
                    let aliases = aliases_of(project);
                    if !aliases.is_empty() {
                        ui.weak(format!("({})", aliases.join(", ")));
                    }
                });
            }
        });
}

/// 右ペイン: 選択中プロジェクトのエイリアス・優先度・スコープ・Area Path。
fn show_detail_panel(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    let Some((organization, project)) = picker.selected.clone() else {
        ui.weak("Select a checked project on the left to edit its details.");
        return;
    };
    ui.strong(format!("{organization}/{project}"));
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Aliases");
        ui.add(
            egui::TextEdit::singleline(&mut picker.aliases_text)
                .hint_text("comma, separated")
                .desired_width(220.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Priority");
        ui.add(egui::TextEdit::singleline(&mut picker.priority_text).desired_width(60.0));
        ui.weak("Lower sorts first");
    });
    picker.commit_text_fields();

    ui.add_space(6.0);
    ui.label("Sync");
    let current = picker.projects.iter().find(|entry| {
        entry.organization.eq_ignore_ascii_case(&organization) && entry.project == project
    });
    let mut include_pr = current.is_some_and(|entry| entry.include_pull_requests);
    let mut include_pipelines = current.is_some_and(|entry| entry.include_pipelines);
    let mut include_wit = current.is_some_and(|entry| entry.include_work_items);
    ui.horizontal(|ui| {
        if ui.checkbox(&mut include_pr, "Pull Requests").changed() {
            picker.set_scope(&organization, &project, Scope::PullRequests, include_pr);
        }
        if ui.checkbox(&mut include_pipelines, "Pipelines").changed() {
            picker.set_scope(&organization, &project, Scope::Pipelines, include_pipelines);
        }
        if ui.checkbox(&mut include_wit, "Work Items").changed() {
            picker.set_scope(&organization, &project, Scope::WorkItems, include_wit);
        }
    });

    ui.add_space(6.0);
    show_area_path_picker(ui, picker);

    ui.add_space(6.0);
    show_repository_picker(ui, picker);
}

/// リポジトリ一覧: 検索フィルタ + チェックボックス。PR の取得範囲を絞る
/// (`interest_repositories`、空ならプロジェクト全体)。
fn show_repository_picker(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    ui.horizontal(|ui| {
        ui.label("Interest repositories");
        ui.weak("(empty = whole project, affects Pull Requests only)");
        if picker.repository_loading {
            ui.spinner();
        }
    });
    if let Some(error) = &picker.repository_error {
        ui.colored_label(
            egui::Color32::RED,
            format!("Could not load repositories: {error}"),
        );
        return;
    }

    let selected_repositories = picker.selected_interest_repositories();
    if !selected_repositories.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for repository in &selected_repositories {
                ui.weak(format!("[{repository}]"));
            }
            if ui.small_button("Clear").clicked() {
                picker.clear_interest_repositories();
            }
        });
    }

    ui.add(
        egui::TextEdit::singleline(&mut picker.repository_filter)
            .hint_text("Filter repositories")
            .desired_width(260.0),
    );

    let filter = picker.repository_filter.to_lowercase();
    let names: Vec<_> = picker
        .repositories
        .iter()
        .filter(|name| filter.is_empty() || name.to_lowercase().contains(&filter))
        .cloned()
        .collect();

    ui.horizontal(|ui| {
        ui.label(format!("{} shown", names.len()));
        if ui.button("Select shown").clicked() {
            for name in &names {
                picker.toggle_interest_repository(name, true);
            }
        }
        if ui.button("Clear shown").clicked() {
            for name in &names {
                picker.toggle_interest_repository(name, false);
            }
        }
    });

    egui::ScrollArea::vertical()
        .id_salt("azure_repository_list")
        .max_height(200.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if names.is_empty() && !picker.repository_loading {
                ui.weak("No repositories found (or none match the filter).");
            }
            for name in &names {
                let mut checked = selected_repositories.contains(name);
                if ui.checkbox(&mut checked, name).changed() {
                    picker.toggle_interest_repository(name, checked);
                }
            }
        });
}

/// Area Path ツリー: 検索フィルタ + インデント付きチェックボックス。
fn show_area_path_picker(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    ui.horizontal(|ui| {
        ui.label("Interest areas");
        ui.weak("(empty = whole project)");
        if picker.area_loading {
            ui.spinner();
        }
    });
    if let Some(error) = &picker.area_error {
        ui.colored_label(egui::Color32::RED, format!("Could not load areas: {error}"));
        return;
    }

    let selected_areas = picker.selected_interest_areas();
    if !selected_areas.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for area in &selected_areas {
                ui.weak(format!("[{area}]"));
            }
            if ui.small_button("Clear").clicked() {
                picker.clear_interest_areas();
            }
        });
    }

    show_area_suggestions(ui, picker, &selected_areas);

    ui.add(
        egui::TextEdit::singleline(&mut picker.area_filter)
            .hint_text("Filter area paths")
            .desired_width(260.0),
    );

    let filter = picker.area_filter.to_lowercase();
    let nodes: Vec<_> = picker
        .area_nodes
        .iter()
        .filter(|node| filter.is_empty() || node.path.to_lowercase().contains(&filter))
        .cloned()
        .collect();

    ui.horizontal(|ui| {
        ui.label(format!("{} shown", nodes.len()));
        if ui.button("Select shown").clicked() {
            for node in &nodes {
                picker.toggle_interest_area(&node.path, true);
            }
        }
        if ui.button("Clear shown").clicked() {
            for node in &nodes {
                picker.toggle_interest_area(&node.path, false);
            }
        }
    });

    egui::ScrollArea::vertical()
        .id_salt("azure_area_tree")
        .max_height(260.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if nodes.is_empty() && !picker.area_loading {
                ui.weak("No areas found (or none match the filter).");
            }
            for node in &nodes {
                let mut checked = selected_areas.contains(&node.path);
                ui.horizontal(|ui| {
                    ui.add_space(node.depth as f32 * 16.0);
                    let name = node.path.rsplit('\\').next().unwrap_or(&node.path);
                    if ui.checkbox(&mut checked, name).changed() {
                        picker.toggle_interest_area(&node.path, checked);
                    }
                });
            }
        });
}

/// 自分に割り当てられた Work Item から集計した Area Path 候補。
/// ボタンを押すまでは何も取得しない (明示的な操作のみ API を呼ぶ)。
fn show_area_suggestions(
    ui: &mut egui::Ui,
    picker: &mut AzureProjectPicker,
    selected_areas: &std::collections::BTreeSet<String>,
) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !picker.area_suggestion_loading,
                egui::Button::new("Suggest from my assigned work items"),
            )
            .clicked()
        {
            picker.suggest_areas_from_my_work_items();
        }
        if picker.area_suggestion_loading {
            ui.spinner();
        }
    });
    if let Some(error) = &picker.area_suggestion_error {
        ui.weak(error);
    }
    if picker.area_suggestions.is_empty() {
        return;
    }
    ui.weak("Suggested (from your assigned work items):");
    egui::ScrollArea::vertical()
        .id_salt("azure_area_suggestions")
        .max_height(120.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (path, count) in picker.area_suggestions.clone() {
                let mut checked = selected_areas.contains(&path);
                if ui
                    .checkbox(&mut checked, format!("{path} ({count})"))
                    .changed()
                {
                    picker.toggle_interest_area(&path, checked);
                }
            }
        });
    ui.separator();
}
