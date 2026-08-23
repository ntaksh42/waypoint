//! 追加種類選択・移動・削除・未保存終了の各確認ダイアログの描画。

use eframe::egui;

use super::app::SettingsApp;
use super::keys::{dialog_keys, lock_modal_focus};

impl SettingsApp {
    pub(super) fn show_confirmations(&mut self, ctx: &egui::Context) {
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
