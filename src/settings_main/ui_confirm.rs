//! 追加種類選択・移動・削除・未保存終了の各確認ダイアログの描画。

use eframe::egui;

use super::app::SettingsApp;
use super::drafts::{DraftKind, MoveToMenuDraft};
use super::keys::{dialog_keys, lock_modal_focus};

impl SettingsApp {
    pub(super) fn show_confirmations(&mut self, ctx: &egui::Context) {
        if self.add_pending {
            let window = egui::Window::new("Add favorite")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if ui.button("Folder").clicked() {
                        self.begin_add(DraftKind::Folder);
                        self.add_pending = false;
                    }
                    if ui.button("File").clicked() {
                        self.begin_add(DraftKind::File);
                        self.add_pending = false;
                    }
                    if ui.button("Special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
                        self.add_pending = false;
                    }
                    if ui.button("Shell location").clicked() {
                        self.begin_add(DraftKind::Shell);
                        self.add_pending = false;
                    }
                    if ui.button("Menu").clicked() {
                        self.begin_add(DraftKind::Submenu);
                        self.add_pending = false;
                    }
                    if ui.button("Separator").clicked() {
                        self.begin_add(DraftKind::Separator);
                        self.add_pending = false;
                    }
                    if ui.button("Folder structure...").clicked() {
                        self.open_import();
                        self.add_pending = false;
                    }
                    if ui.button("My Special Folders preset").clicked() {
                        self.insert_my_special_folders_preset();
                        self.add_pending = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.add_pending = false;
                    }
                });
            lock_modal_focus(ctx, &window);
            // 種類を選ぶ画面で既定の 1 つは決められない。Esc の取り消しだけ受ける
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.add_pending = false;
            }
        }

        if self.move_pending {
            let window = egui::Window::new("Move favorite")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Move up").clicked() {
                            self.move_selected(-1);
                            self.move_pending = false;
                        }
                        if ui.button("Move down").clicked() {
                            self.move_selected(1);
                            self.move_pending = false;
                        }
                        if ui.button("Move to menu...").clicked() {
                            self.move_to_menu_draft = Some(MoveToMenuDraft::default());
                            self.move_pending = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.move_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // 上下どちらも既定にはできない。Esc の取り消しだけ受ける
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.move_pending = false;
            }
        }

        if self.delete_pending {
            let window = egui::Window::new("Remove item?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("{} will be removed.", self.selection_summary()));
                    ui.horizontal(|ui| {
                        if ui.button("Remove").clicked() {
                            self.remove_selected();
                            self.delete_pending = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.delete_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // Delete で出した確認なので Enter は削除に割り当てる
            let (accept, dismiss) = dialog_keys(ctx, true);
            if accept {
                self.remove_selected();
                self.delete_pending = false;
            } else if dismiss {
                self.delete_pending = false;
            }
        }

        if self.close_pending {
            let window = egui::Window::new("Discard changes?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("There are unsaved changes.");
                    ui.horizontal(|ui| {
                        if ui.button("Discard").clicked() {
                            self.dirty = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.close_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // 破棄は取り返しがつかない。Enter には割り当てず、Esc で編集へ戻す
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.close_pending = false;
            }
        }
    }
}
