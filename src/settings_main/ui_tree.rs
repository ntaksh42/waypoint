//! 左ペインのメニューツリー描画。
//!
//! `menu_choices` が返すフラットなパスリストを、パスの深さに応じた
//! インデント付き選択リストとして描く。エクスプローラのフォルダツリーに
//! 近い見た目にすることで、階層が深くても現在地が視覚的に分かるようにする。

use eframe::egui;

use super::app::SettingsApp;
use super::helpers::{items_at, menu_choices};

impl SettingsApp {
    /// 左ペイン: メニュー階層をツリーとして表示し、選ぶと右の一覧を切り替える。
    pub(super) fn show_menu_tree(&mut self, ui: &mut egui::Ui) {
        ui.label("Menus");
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("menu_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (path, name) in menu_choices(&self.config) {
                    // "Parent > Child" の最後の区間だけを表示名にし、深さはインデントで示す。
                    let depth = path.len();
                    let label = name.rsplit(" > ").next().unwrap_or(&name);
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 14.0);
                        if ui
                            .selectable_label(path == self.selected_menu, label)
                            .clicked()
                        {
                            self.switch_menu(path);
                        }
                    });
                }
            });
    }

    /// 別のメニュー階層へ切り替える。選択状態は切り替え先の先頭項目にリセットする。
    fn switch_menu(&mut self, path: Vec<usize>) {
        let first =
            items_at(&self.config, &path).and_then(|items| (!items.is_empty()).then_some(0));
        self.selected_items = first.into_iter().collect();
        self.active_item = first;
        self.selection_anchor = first;
        self.selected_menu = path;
    }
}
