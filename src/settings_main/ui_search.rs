//! 全メニュー横断の項目検索結果一覧 (FR-6.14)。
//!
//! 検索ボックスに入力がある間、通常の項目一覧の代わりにこちらを描く。
//! 行をクリックすると該当メニューへ切り替えて選択し、検索を閉じて
//! 通常表示へ戻る。並べ替えや複数選択など編集系の操作はここでは行わず、
//! 「見つけて飛ぶ」ことだけに専念させている。

use eframe::egui;

use super::app::SettingsApp;
use super::helpers::search_items;
use super::{COL_KIND, COL_MENU, COL_NAME};

/// 一度に描く検索結果の上限。大量ヒット時に一覧の描画コストが
/// 際限なく増えないための安全弁 (絞り込みを促す)。
const MAX_SEARCH_RESULTS: usize = 200;

impl SettingsApp {
    pub(super) fn show_search_results(&mut self, ui: &mut egui::Ui, max_height: f32) {
        let mut hits = search_items(&self.config, &self.search);
        let total = hits.len();
        let truncated = total > MAX_SEARCH_RESULTS;
        hits.truncate(MAX_SEARCH_RESULTS);

        let mut jump = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| ui.strong("Name"),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(COL_KIND, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| ui.strong("Type"),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(COL_MENU, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| ui.strong("Menu"),
                );
                ui.strong("Location or content");
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("search_results")
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if hits.is_empty() {
                        ui.weak("No items match.");
                    }
                    for hit in &hits {
                        ui.horizontal(|ui| {
                            let clicked = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| ui.selectable_label(false, &hit.label),
                                )
                                .inner
                                .clicked();
                            ui.allocate_ui_with_layout(
                                egui::vec2(COL_KIND, ui.spacing().interact_size.y),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| ui.label(hit.kind),
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(COL_MENU, ui.spacing().interact_size.y),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| ui.label(&hit.menu_name),
                            );
                            ui.label(&hit.detail);
                            if clicked {
                                jump = Some((hit.menu_path.clone(), hit.index));
                            }
                        });
                    }
                });
        });

        if let Some((path, index)) = jump {
            self.jump_to_item(path, index);
        } else if truncated {
            ui.weak(format!(
                "Showing first {MAX_SEARCH_RESULTS} of {total} matches. Refine your search to narrow down."
            ));
        } else {
            ui.weak(format!("{total} match(es)."));
        }
    }
}
