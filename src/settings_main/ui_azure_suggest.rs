//! Azure DevOps 設定: 直近アクティビティからの優先度提案モーダル。
//!
//! 監視中プロジェクトを直近 90 日の担当・メンション件数で並べ、
//! チェックした分だけ優先度と興味 Area へ書き戻す。詳細ペインは
//! `ui_azure_detail`、ウィンドウ骨格は `ui_azure` にある。

use eframe::egui;
use waypoint::azure_devops::{AreaNode, ProjectActivity};

use super::azure_draft::AzureProjectPicker;
use super::keys::{dialog_keys, lock_modal_focus};
use super::ui_azure::truncate_for_row;
use super::ui_azure_detail::{SUGGESTION_LIST_HEIGHT, leaf_of, show_bulk_row, show_load_error};

/// 直近アクティビティからの Project / Area 優先度提案を表示する専用モーダル。
/// フィルタで絞り込み、チェックした行だけ Apply で確定する
/// (`azure_draft::apply_priority_suggestions` が実際の書き戻しを行う)。
pub(super) fn show_priority_suggestion_modal(ctx: &egui::Context, picker: &mut AzureProjectPicker) {
    if !picker.priority_suggestion_open {
        return;
    }
    let mut apply = false;
    let mut cancel = false;
    let (host_width, host_height) = ctx.input(|input| {
        input
            .viewport()
            .inner_rect
            .map_or((1100.0, 760.0), |rect| (rect.width(), rect.height()))
    });
    let max_width = (host_width - 60.0).max(360.0);
    let max_height = (host_height - 60.0).max(300.0);
    let window = egui::Window::new("Suggest priorities from recent activity")
        .collapsible(false)
        .resizable(true)
        .default_size([max_width.min(620.0), max_height.min(560.0)])
        .min_size([max_width.min(380.0), max_height.min(300.0)])
        .max_size([max_width, max_height])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // 一覧の伸縮でフッターが押し出されないよう、パネルで先に確保する。
            egui::Panel::bottom("azure_priority_footer")
                .show_separator_line(false)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    top: 6,
                    ..Default::default()
                }))
                .show(ui, |ui| {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.weak(
                            "Checked projects are ranked by activity. \
                             Expand a project to pick its areas.",
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            cancel = ui.button("Cancel").clicked();
                            apply = ui
                                .add_enabled(
                                    !picker.priority_suggestion_checked.is_empty(),
                                    egui::Button::new("Apply"),
                                )
                                .clicked();
                        });
                    });
                });

            egui::Panel::top("azure_priority_header")
                .show_separator_line(false)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    bottom: 6,
                    ..Default::default()
                }))
                .show(ui, |ui| {
                    ui.weak("Ranked by assignments and @mentions in the last 90 days.");
                    if let Some(error) = &picker.priority_suggestion_error {
                        ui.colored_label(egui::Color32::RED, truncate_for_row(error, 110))
                            .on_hover_text(error);
                    }
                    let width = (ui.available_width() * 0.5).clamp(160.0, 320.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut picker.priority_suggestion_filter)
                            .hint_text("Filter by project name")
                            .desired_width(width),
                    );
                });

            let filter = picker.priority_suggestion_filter.to_lowercase();
            let shown: Vec<_> = picker
                .priority_suggestions
                .iter()
                .filter(|entry| filter.is_empty() || entry.project.to_lowercase().contains(&filter))
                .cloned()
                .collect();

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.weak(format!(
                            "{} of {} shown; {} checked",
                            shown.len(),
                            picker.priority_suggestions.len(),
                            picker.priority_suggestion_checked.len()
                        ));
                        if ui.button("Check shown").clicked() {
                            for entry in &shown {
                                picker
                                    .priority_suggestion_checked
                                    .insert((entry.organization.clone(), entry.project.clone()));
                            }
                        }
                        if ui.button("Uncheck shown").clicked() {
                            for entry in &shown {
                                picker
                                    .priority_suggestion_checked
                                    .remove(&(entry.organization.clone(), entry.project.clone()));
                            }
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("azure_priority_suggestion_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if shown.is_empty() {
                                ui.weak("No projects match the filter.");
                            }
                            for entry in &shown {
                                show_priority_suggestion_row(ui, picker, entry);
                            }
                        });
                });
        });
    lock_modal_focus(ctx, &window);
    let (_, dismiss) = dialog_keys(ctx, false);
    cancel |= dismiss;
    if apply {
        picker.apply_priority_suggestions();
    } else if cancel {
        picker.priority_suggestion_open = false;
    }
}

/// 提案モーダル 1 行分: チェックボックス + プロジェクト名 + 活動件数 +
/// 展開ボタン。展開すると、そのプロジェクトの Area ツリー全体を
/// (ルートから子階層まで) チェックボックス付きで表示する。
fn show_priority_suggestion_row(
    ui: &mut egui::Ui,
    picker: &mut AzureProjectPicker,
    entry: &ProjectActivity,
) {
    let key = (entry.organization.clone(), entry.project.clone());
    let mut checked = picker.priority_suggestion_checked.contains(&key);
    let is_expanded = picker.priority_suggestion_expanded.as_ref() == Some(&key);
    ui.horizontal(|ui| {
        if ui.checkbox(&mut checked, "").changed() {
            if checked {
                picker.priority_suggestion_checked.insert(key.clone());
            } else {
                picker.priority_suggestion_checked.remove(&key);
            }
        }
        // 展開ボタンを先に右端へ置き、残りを名前に使わせる。左から流すと
        // 長いプロジェクト名でボタンが行外へ出て押せなくなる。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle_label = if is_expanded {
                "▼ Areas"
            } else {
                "▶ Areas"
            };
            if ui.small_button(toggle_label).clicked() {
                picker.toggle_priority_suggestion_expanded(&entry.organization, &entry.project);
            }
            ui.weak(format!("{} item(s)", entry.count));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&entry.project).strong()).truncate())
                    .on_hover_text(&entry.project);
            });
        });
    });
    if !is_expanded {
        return;
    }
    ui.indent(("azure_priority_suggestion_areas", &key), |ui| {
        show_priority_suggestion_area_tree(ui, picker, &entry.organization, &entry.project);
    });
}

/// 展開中プロジェクトの Area ツリー。既存の詳細パネル (`show_area_path_picker`)
/// と同じ「ルートからの深さでインデントしたチェックボックス」表示だが、
/// こちらは複数プロジェクトを同時に展開できるようキーをプロジェクト単位で持つ。
fn show_priority_suggestion_area_tree(
    ui: &mut egui::Ui,
    picker: &mut AzureProjectPicker,
    organization: &str,
    project: &str,
) {
    let key = (organization.to_string(), project.to_string());
    if picker.priority_suggestion_area_loading.as_ref() == Some(&key) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("Loading area tree...");
        });
        return;
    }
    if let Some(error) = &picker.priority_suggestion_area_error {
        show_load_error(ui, "Could not load areas", &error.clone());
        return;
    }
    let Some(nodes) = picker.priority_suggestion_area_trees.get(&key).cloned() else {
        return;
    };
    if nodes.is_empty() {
        ui.weak("No areas found for this project.");
        return;
    }
    let selected_areas = picker
        .find_project(organization, project)
        .map(|entry| entry.interest_areas.clone())
        .unwrap_or_default();

    let width = (ui.available_width() - 8.0).clamp(80.0, 240.0);
    ui.add(
        egui::TextEdit::singleline(&mut picker.priority_suggestion_area_filter)
            .hint_text("Filter areas")
            .desired_width(width),
    );
    let filter = picker.priority_suggestion_area_filter.to_lowercase();
    let filtering = !filter.is_empty();
    let shown = filter_area_nodes(&nodes, &filter);

    if let Some(select) = show_bulk_row(ui, shown.len()) {
        for node in &shown {
            picker.toggle_priority_suggestion_area(organization, project, &node.path, select);
        }
    }

    egui::ScrollArea::vertical()
        .id_salt(("azure_priority_suggestion_area_scroll", &key))
        .max_height(SUGGESTION_LIST_HEIGHT + 60.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.weak("No areas match the filter.");
            }
            for node in &shown {
                let mut checked = selected_areas.contains(&node.path);
                ui.horizontal(|ui| {
                    // 絞り込み中は親が落ちるので、インデントすると階層が
                    // ずれて見える。代わりに全パスを出して所属を示す。
                    let label = if filtering {
                        trim_project_prefix(&node.path, project)
                    } else {
                        ui.add_space(node.depth as f32 * 16.0);
                        leaf_of(&node.path)
                    };
                    if ui
                        .checkbox(&mut checked, label)
                        .on_hover_text(&node.path)
                        .changed()
                    {
                        picker.toggle_priority_suggestion_area(
                            organization,
                            project,
                            &node.path,
                            checked,
                        );
                    }
                });
            }
        });
}

/// Area ツリーを絞り込む。`filter` は小文字化済みを渡すこと。
/// 空なら全件を返す。フルパスに対して部分一致を見るので、親の名前でも
/// 子孫が拾える (`Launcher` で `Waypoint\Launcher\Menu` が出る)。
fn filter_area_nodes(nodes: &[AreaNode], filter: &str) -> Vec<AreaNode> {
    nodes
        .iter()
        .filter(|node| filter.is_empty() || node.path.to_lowercase().contains(filter))
        .cloned()
        .collect()
}

/// `Project\Parent\Leaf` から先頭のプロジェクト名を落として
/// `Parent\Leaf` にする。絞り込み結果の行はどれも同じプロジェクトなので、
/// 先頭を繰り返しても幅を食うだけで区別に効かない。
fn trim_project_prefix<'a>(path: &'a str, project: &str) -> &'a str {
    path.strip_prefix(project)
        .and_then(|rest| rest.strip_prefix('\\'))
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::{AreaNode, filter_area_nodes, trim_project_prefix};

    fn tree() -> Vec<AreaNode> {
        [
            ("Waypoint\\Launcher", 1),
            ("Waypoint\\Launcher\\Menu", 2),
            ("Waypoint\\Settings", 1),
        ]
        .into_iter()
        .map(|(path, depth)| AreaNode {
            path: path.to_string(),
            depth,
        })
        .collect()
    }

    #[test]
    fn an_empty_filter_keeps_every_node() {
        assert_eq!(filter_area_nodes(&tree(), "").len(), 3);
    }

    #[test]
    fn a_parent_name_also_matches_its_descendants() {
        // フルパス一致なので、親を打つと配下がまとめて出る。
        let shown = filter_area_nodes(&tree(), "launcher");
        let paths: Vec<_> = shown.iter().map(|node| node.path.as_str()).collect();
        assert_eq!(paths, ["Waypoint\\Launcher", "Waypoint\\Launcher\\Menu"]);
    }

    #[test]
    fn matching_is_case_insensitive_against_a_lowercased_filter() {
        let shown = filter_area_nodes(&tree(), "settings");
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].path, "Waypoint\\Settings");
    }

    #[test]
    fn a_filter_that_matches_nothing_yields_an_empty_list() {
        assert!(filter_area_nodes(&tree(), "nope").is_empty());
    }

    #[test]
    fn drops_the_leading_project_segment() {
        assert_eq!(
            trim_project_prefix("Waypoint\\Launcher\\Menu", "Waypoint"),
            "Launcher\\Menu"
        );
    }

    #[test]
    fn keeps_the_path_when_the_project_is_not_the_first_segment() {
        // 区切りを跨いだ部分一致で切らないこと (Waypoint2 は Waypoint で
        // 始まるが別プロジェクト)。
        assert_eq!(
            trim_project_prefix("Waypoint2\\Launcher", "Waypoint"),
            "Waypoint2\\Launcher"
        );
    }

    #[test]
    fn keeps_the_root_node_itself() {
        // ルート (プロジェクト名だけ) は削ると空になるので、そのまま残す。
        assert_eq!(trim_project_prefix("Waypoint", "Waypoint"), "Waypoint");
    }
}
