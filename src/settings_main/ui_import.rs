//! フォルダ構造インポート画面の描画。

use eframe::egui;

use super::app::SettingsApp;
use super::helpers::show_import_node;
use super::keys::{dialog_keys, lock_modal_focus};

impl SettingsApp {
    pub(super) fn show_import_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.import_draft.as_mut() else {
            return;
        };
        let mut refresh = false;
        let mut apply = false;
        let mut cancel = false;

        let window = egui::Window::new("Import folder structure")
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Root folder");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut draft.root).desired_width(460.0));
                    if ui.button("Browse...").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        draft.root = path.display().to_string();
                        refresh = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Subfolder depth");
                    ui.add(egui::DragValue::new(&mut draft.depth).range(0..=10));
                    if ui.button("Preview").clicked() {
                        refresh = true;
                    }
                });
                ui.weak("Depth 0 imports only the selected folder. This is a one-time import.");

                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                if let Some(preview) = draft.preview.as_mut() {
                    ui.label(format!(
                        "Preview: {} selected folder(s). Rename or clear items before importing.",
                        preview.included_count()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(330.0)
                        .show(ui, |ui| show_import_node(ui, preview, 0));
                } else {
                    ui.weak("Choose a folder and select Preview.");
                }

                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(
                            draft
                                .preview
                                .as_ref()
                                .is_some_and(|preview| preview.included_count() > 0),
                            egui::Button::new("Import"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        if refresh {
            let root = std::path::Path::new(draft.root.trim());
            match waypoint::folder_import::scan(root, draft.depth) {
                Ok(preview) => {
                    draft.preview = Some(preview);
                    draft.error = None;
                }
                Err(error) => {
                    draft.preview = None;
                    draft.error = Some(format!("Could not read folder: {error}"));
                }
            }
        }

        // 取り込みは Preview が済んでいるときだけ Enter で確定できる
        let (accept, dismiss) = dialog_keys(ctx, true);
        let apply = apply
            || (accept
                && draft
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.included_count() > 0));
        let cancel = cancel || dismiss;
        if apply {
            let item = self
                .import_draft
                .as_ref()
                .and_then(|draft| draft.preview.as_ref())
                .and_then(waypoint::folder_import::FolderNode::to_item);
            if let Some(item) = item {
                if let Some(items) = self.current_items_mut() {
                    items.push(item);
                    let index = items.len() - 1;
                    self.select_single(index);
                    self.dirty = true;
                    self.status = None;
                }
                self.import_draft = None;
            }
        } else if cancel {
            self.import_draft = None;
        }
    }
}
