//! 上部ツールバー: 作成・編集・整理を機能グループに分けて 1 列に並べる。
//! メニューバー (File/Favorite/Options) の代わりに、頻度の高い操作を
//! ボタンとして常時見せ、設定系だけを歯車ドロップダウンへ寄せる。

use eframe::egui;

use super::app::SettingsApp;
use super::drafts::{DraftKind, MoveToMenuDraft};

impl SettingsApp {
    pub(super) fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        // 編集ダイアログ等が非モーダルの egui::Window なので、開いたまま
        // 背後のツールバーをクリックできてしまう。放置すると、例えば
        // 編集ダイアログを開いたまま別項目を Remove した後に OK を押すと
        // 削除で詰まった index が別の項目を上書きする。ダイアログが
        // 何か開いている間はツールバー全体を無効化して塞ぐ。
        //
        // 検索中も同様に塞ぐ: 一覧が検索結果 (全メニュー横断) に
        // 差し替わっており、selected_items/selected_menu は検索を始める前の
        // ままなので、ここで Edit 等を押すと画面に見えている項目とは
        // 無関係な項目を操作してしまう。
        let dialog_open = self.any_dialog_open();
        let searching = !self.search.trim().is_empty();
        ui.add_enabled_ui(!dialog_open && !searching, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("add_item_menu")
                    .selected_text("Add")
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        if ui.button("Folder").clicked() {
                            self.begin_add(DraftKind::Folder);
                        }
                        if ui.button("File").clicked() {
                            self.begin_add(DraftKind::File);
                        }
                        if ui.button("Special folder").clicked() {
                            self.begin_add(DraftKind::SpecialFolder);
                        }
                        if ui.button("Shell location").clicked() {
                            self.begin_add(DraftKind::Shell);
                        }
                        if ui.button("Submenu").clicked() {
                            self.begin_add(DraftKind::Submenu);
                        }
                        if ui.button("Separator").clicked() {
                            self.begin_add(DraftKind::Separator);
                        }
                        ui.separator();
                        if ui.button("Import folder structure...").clicked() {
                            self.open_import();
                        }
                        if ui.button("My Special Folders preset").clicked() {
                            self.insert_my_special_folders_preset();
                        }
                    });

                ui.separator();

                let selected = !self.selected_items.is_empty();
                let multi = self.selected_items.len() > 1;
                if ui
                    .add_enabled(
                        selected,
                        egui::Button::new(if multi { "Edit all..." } else { "Edit" }),
                    )
                    .on_hover_text(if multi {
                        "Edit shared properties (Open mode, Show Git branch) for all selected items"
                    } else {
                        "Edit this item"
                    })
                    .clicked()
                {
                    self.begin_edit();
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Remove"))
                    .on_hover_text("Remove selected items (Delete)")
                    .clicked()
                {
                    self.delete_pending = true;
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Duplicate"))
                    .on_hover_text("Duplicate selected items (Ctrl+D)")
                    .clicked()
                {
                    self.duplicate_selected();
                }

                ui.separator();

                if ui
                    .add_enabled(selected, egui::Button::new("Copy"))
                    .on_hover_text("Copy selected items to clipboard (Ctrl+C)")
                    .clicked()
                {
                    self.copy_selected();
                }
                if ui
                    .add_enabled(!self.clipboard.is_empty(), egui::Button::new("Paste"))
                    .on_hover_text("Paste clipboard items here (Ctrl+V)")
                    .clicked()
                {
                    self.paste_clipboard();
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Move to menu..."))
                    .on_hover_text("Move selected items to another menu")
                    .clicked()
                {
                    self.move_to_menu_draft = Some(MoveToMenuDraft::default());
                }

                ui.separator();

                egui::ComboBox::from_id_salt("settings_menu")
                    .selected_text("\u{2699} Settings")
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        if ui.button("Variables...").clicked() {
                            self.open_variables();
                        }
                        if ui.button("Trigger...").clicked() {
                            self.open_trigger();
                        }
                        if ui.button("Azure DevOps...").clicked() {
                            self.open_azure_settings();
                        }
                    });
            });
        });
    }
}
