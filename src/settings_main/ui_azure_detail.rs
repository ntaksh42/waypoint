//! Azure DevOps 設定画面の右ペイン (選択中プロジェクトの詳細)。
//!
//! ウィンドウ骨格・接続行・プロジェクト一覧は `ui_azure`、
//! 優先度提案モーダルは `ui_azure_suggest` にある。

use eframe::egui;

use super::azure_draft::{AzureProjectPicker, Scope};
use super::ui_azure::truncate_for_row;

/// 右ペインのリスト (Area / Repository / 候補) の高さ。
/// 詳細ペイン全体が 1 つの `ScrollArea` の中にあるので、ここは
/// 親の残り高さではなく固定値で区切る。3 つ並べても縦スクロールで
/// 全部に届く範囲に収める。
const AREA_LIST_HEIGHT: f32 = 220.0;
const REPOSITORY_LIST_HEIGHT: f32 = 180.0;
pub(super) const SUGGESTION_LIST_HEIGHT: f32 = 120.0;
/// 入力欄・フィルタ欄の幅の上限。親が狭いときは親に追従する。
const FIELD_MAX_WIDTH: f32 = 260.0;

/// 親の幅に収まる入力欄幅。固定幅だと狭いペインで右端が切れる。
fn field_width(ui: &egui::Ui, max: f32) -> f32 {
    (ui.available_width() - 8.0).clamp(80.0, max)
}

/// Area Path (`Project\Parent\Leaf`) の末尾要素。
pub(super) fn leaf_of(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// 読み込み失敗の一行表示。全文はホバーで見せる (長いエラーをそのまま
/// 流すとペイン幅を溢れるため)。
pub(super) fn show_load_error(ui: &mut egui::Ui, prefix: &str, error: &str) {
    ui.colored_label(
        egui::Color32::RED,
        truncate_for_row(&format!("{prefix}: {error}"), 100),
    )
    .on_hover_text(error);
}

/// 選択済みの一覧を短いチップとして並べ、右端に一括解除を置く。
/// `label_of` で表示名を決める (Area Path は末尾だけ出す)。
fn show_selected_chips<'a>(
    ui: &mut egui::Ui,
    selected: impl IntoIterator<Item = &'a String>,
    label_of: impl Fn(&str) -> &str,
    clear: &mut dyn FnMut(),
) {
    let mut any = false;
    ui.horizontal_wrapped(|ui| {
        for value in selected {
            any = true;
            ui.weak(format!("[{}]", label_of(value)))
                .on_hover_text(value);
        }
        if any && ui.small_button("Clear").clicked() {
            clear();
        }
    });
}

/// 件数表示と「表示中を全選択 / 全解除」。両ピッカーで同じ形。
/// 押されたときは `Some(true)` / `Some(false)` を返す。
pub(super) fn show_bulk_row(ui: &mut egui::Ui, shown: usize) -> Option<bool> {
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!("{shown} shown"));
        if ui.button("Select shown").clicked() {
            result = Some(true);
        }
        if ui.button("Clear shown").clicked() {
            result = Some(false);
        }
    });
    result
}

/// 右ペイン: 選択中プロジェクトのエイリアス・優先度・スコープ・Area Path。
pub(super) fn show_detail_panel(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    let Some((organization, project)) = picker.selected.clone() else {
        ui.weak("Select a checked project on the left to edit its details.");
        return;
    };
    let title = format!("{organization}/{project}");
    ui.add(egui::Label::new(egui::RichText::new(&title).strong()).truncate())
        .on_hover_text(&title);
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Aliases");
        let width = field_width(ui, FIELD_MAX_WIDTH);
        ui.add(
            egui::TextEdit::singleline(&mut picker.aliases_text)
                .hint_text("comma, separated")
                .desired_width(width),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Priority");
        ui.add(egui::TextEdit::singleline(&mut picker.priority_text).desired_width(60.0));
        ui.weak("Lower sorts first");
    });
    picker.commit_text_fields();

    ui.add_space(6.0);
    ui.label("Sync");
    let current = picker.projects.iter().find(|entry| {
        entry.organization.eq_ignore_ascii_case(&organization)
            && entry.project.eq_ignore_ascii_case(&project)
    });
    let mut include_pr = current.is_some_and(|entry| entry.include_pull_requests);
    let mut include_pipelines = current.is_some_and(|entry| entry.include_pipelines);
    let mut include_wit = current.is_some_and(|entry| entry.include_work_items);
    // 狭いペインでは 3 つ並ばないので折り返す。
    ui.horizontal_wrapped(|ui| {
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
    egui::CollapsingHeader::new("Interest repositories")
        .id_salt("azure_repositories")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.weak("(empty = whole project, affects Pull Requests only)");
                if picker.repository_loading {
                    ui.spinner();
                }
            });
            if let Some(error) = &picker.repository_error {
                show_load_error(ui, "Could not load repositories", &error.clone());
                return;
            }

            let selected_repositories = picker.selected_interest_repositories();
            let mut clear_requested = false;
            show_selected_chips(ui, &selected_repositories, |name| name, &mut || {
                clear_requested = true
            });
            if clear_requested {
                picker.clear_interest_repositories();
            }

            let width = field_width(ui, FIELD_MAX_WIDTH);
            ui.add(
                egui::TextEdit::singleline(&mut picker.repository_filter)
                    .hint_text("Filter repositories")
                    .desired_width(width),
            );

            let filter = picker.repository_filter.to_lowercase();
            let names: Vec<_> = picker
                .repositories
                .iter()
                .filter(|name| filter.is_empty() || name.to_lowercase().contains(&filter))
                .cloned()
                .collect();

            if let Some(select) = show_bulk_row(ui, names.len()) {
                for name in &names {
                    picker.toggle_interest_repository(name, select);
                }
            }

            egui::ScrollArea::vertical()
                .id_salt("azure_repository_list")
                .max_height(REPOSITORY_LIST_HEIGHT)
                // 縦は縮ませる。`false` に固定すると、候補が 0 件でも
                // max_height ぶんの空白が居座り、下の節との間が間延びする。
                .auto_shrink([false, true])
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
        });
}

/// Area Path ツリー: 検索フィルタ + インデント付きチェックボックス。
fn show_area_path_picker(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    egui::CollapsingHeader::new("Interest areas")
        .id_salt("azure_areas")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.weak("(empty = whole project)");
                if picker.area_loading {
                    ui.spinner();
                }
            });
            if let Some(error) = &picker.area_error {
                show_load_error(ui, "Could not load areas", &error.clone());
                return;
            }

            let selected_areas = picker.selected_interest_areas();
            // Area Path はルートからの全パスで長い。末尾だけ出し、全体は
            // ホバーで見せる (以前はそのまま並べていたので、数個選ぶだけで
            // ペイン幅を溢れていた)。
            let mut clear_requested = false;
            show_selected_chips(ui, &selected_areas, leaf_of, &mut || {
                clear_requested = true;
            });
            if clear_requested {
                picker.clear_interest_areas();
            }

            show_area_suggestions(ui, picker, &selected_areas);

            let width = field_width(ui, FIELD_MAX_WIDTH);
            ui.add(
                egui::TextEdit::singleline(&mut picker.area_filter)
                    .hint_text("Filter area paths")
                    .desired_width(width),
            );

            let filter = picker.area_filter.to_lowercase();
            let nodes: Vec<_> = picker
                .area_nodes
                .iter()
                .filter(|node| filter.is_empty() || node.path.to_lowercase().contains(&filter))
                .cloned()
                .collect();

            if let Some(select) = show_bulk_row(ui, nodes.len()) {
                for node in &nodes {
                    picker.toggle_interest_area(&node.path, select);
                }
            }

            egui::ScrollArea::vertical()
                .id_salt("azure_area_tree")
                .max_height(AREA_LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if nodes.is_empty() && !picker.area_loading {
                        ui.weak("No areas found (or none match the filter).");
                    }
                    for node in &nodes {
                        let mut checked = selected_areas.contains(&node.path);
                        ui.horizontal(|ui| {
                            ui.add_space(node.depth as f32 * 16.0);
                            let name = leaf_of(&node.path);
                            if ui.checkbox(&mut checked, name).changed() {
                                picker.toggle_interest_area(&node.path, checked);
                            }
                        });
                    }
                });
        });
}

/// 自分に割り当てられた Work Item から集計した Area Path 候補。
/// ボタンを押すまでは何も取得しない (明示的な操作のみ API を呼ぶ)。
fn show_area_suggestions(
    ui: &mut egui::Ui,
    picker: &mut AzureProjectPicker,
    selected_areas: &std::collections::BTreeSet<String>,
) {
    ui.horizontal_wrapped(|ui| {
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
        ui.weak(truncate_for_row(error, 100)).on_hover_text(error);
    }
    if picker.area_suggestions.is_empty() {
        return;
    }
    ui.weak("Suggested (from your assigned work items):");
    egui::ScrollArea::vertical()
        .id_salt("azure_area_suggestions")
        .max_height(SUGGESTION_LIST_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (path, count) in picker.area_suggestions.clone() {
                let mut checked = selected_areas.contains(&path);
                let leaf = leaf_of(&path);
                if ui
                    .checkbox(&mut checked, format!("{leaf} ({count})"))
                    .on_hover_text(&path)
                    .changed()
                {
                    picker.toggle_interest_area(&path, checked);
                }
            }
        });
    ui.separator();
}
