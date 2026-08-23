//! 項目一覧の描画: ツールバー・一覧テーブル・各行のドラッグ&ドロップ。

use eframe::egui;
use waypoint::config::Item;

use super::app::SettingsApp;
use super::drafts::DraftKind;
use super::helpers::{item_detail, item_kind, item_open, menu_choices};
use super::items_at;
use super::{COL_KIND, COL_NAME, COL_OPEN, MoveToMenuDraft, ROW_INDENT, TAIL_DROP_HEIGHT};

/// ドラッグ中の行。掴んだ時点の添字だけを運ぶ。
#[derive(Debug, Clone, Copy)]
pub(super) struct DragRow {
    pub(super) index: usize,
}

/// 挿入位置を示す線を引く。`after` なら行の下端、そうでなければ上端。
pub(super) fn draw_insert_line(ui: &egui::Ui, rect: egui::Rect, after: bool) {
    let y = if after { rect.bottom() } else { rect.top() };
    let stroke = egui::Stroke::new(2.0, ui.visuals().selection.bg_fill);
    ui.painter().hline(rect.x_range(), y, stroke);
}

/// 「元の並びで `insert_at` の手前」を、取り除いた後の添字へ直す。
///
/// 先に `remove(from)` すると `from` より後ろが 1 つ詰まるため、
/// 挿入位置がそれより後ろなら 1 引く。
pub(super) fn reorder_target(from: usize, insert_at: usize) -> usize {
    if insert_at > from {
        insert_at - 1
    } else {
        insert_at
    }
}

impl SettingsApp {
    pub(super) fn show_items(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default().show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                        ui.close();
                    }
                    if ui.button("Save & Close").clicked() {
                        if self.save() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.close();
                    }
                    if ui.button("Close").clicked() {
                        self.request_close(&ctx);
                        ui.close();
                    }
                });
                ui.menu_button("Favorite", |ui| {
                    if ui.button("Add folder").clicked() {
                        self.begin_add(DraftKind::Folder);
                        ui.close();
                    }
                    if ui.button("Add file").clicked() {
                        self.begin_add(DraftKind::File);
                        ui.close();
                    }
                    if ui.button("Add special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
                        ui.close();
                    }
                    if ui.button("Add shell location").clicked() {
                        self.begin_add(DraftKind::Shell);
                        ui.close();
                    }
                    if ui.button("Add menu").clicked() {
                        self.begin_add(DraftKind::Submenu);
                        ui.close();
                    }
                    if ui.button("Add separator").clicked() {
                        self.begin_add(DraftKind::Separator);
                        ui.close();
                    }
                    if ui.button("Import folder structure...").clicked() {
                        self.open_import();
                        ui.close();
                    }
                    if ui.button("Add My Special Folders preset").clicked() {
                        self.insert_my_special_folders_preset();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(!self.selected_items.is_empty(), egui::Button::new("Edit"))
                        .clicked()
                    {
                        self.begin_edit();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.selected_items.is_empty(),
                            egui::Button::new("Move to menu..."),
                        )
                        .clicked()
                    {
                        self.move_to_menu_draft = Some(MoveToMenuDraft::default());
                        ui.close();
                    }
                });
                ui.menu_button("Options", |ui| {
                    if ui.button("Variables...").clicked() {
                        self.open_variables();
                        ui.close();
                    }
                    if ui.button("Trigger...").clicked() {
                        self.open_trigger();
                        ui.close();
                    }
                });
            });
            ui.separator();

            ui.label("Menu or group to edit:");
            let choices = menu_choices(&self.config);
            let selected_name = choices
                .iter()
                .find(|(path, _)| path == &self.selected_menu)
                .map(|(_, name)| name.as_str())
                .unwrap_or("Main");
            egui::ComboBox::from_id_salt("menu_to_edit")
                .selected_text(selected_name)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (path, name) in choices {
                        if ui
                            .selectable_label(path == self.selected_menu, name)
                            .clicked()
                        {
                            let first = items_at(&self.config, &path)
                                .and_then(|items| (!items.is_empty()).then_some(0));
                            self.selected_items = first.into_iter().collect();
                            self.active_item = first;
                            self.selection_anchor = first;
                            self.selected_menu = path;
                        }
                    }
                });
            ui.add_space(8.0);

            let rows = self.current_items().cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let selected = !self.selected_items.is_empty();
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Up").min_size([34.0, 28.0].into()),
                        )
                        .on_hover_text("Move up (Alt+Up)")
                        .clicked()
                    {
                        self.move_selected(-1);
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Down").min_size([34.0, 28.0].into()),
                        )
                        .on_hover_text("Move down (Alt+Down)")
                        .clicked()
                    {
                        self.move_selected(1);
                    }
                });

                let list_width = (ui.available_width() - 92.0).max(360.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(list_width, 400.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(list_width);
                        ui.set_max_width(list_width);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_size(egui::vec2(list_width - 4.0, 360.0));
                            egui::ScrollArea::both()
                                .min_scrolled_height(360.0)
                                .show(ui, |ui| {
                                    self.show_item_rows(ui, &rows);
                                });
                        });
                        ui.weak("Drag rows to reorder. Drop folders into the list to add them");
                    },
                );

                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 9.0;
                    if ui
                        .add(egui::Button::new("Add").min_size([74.0, 32.0].into()))
                        .clicked()
                    {
                        self.add_pending = true;
                    }
                    if ui
                        .add(egui::Button::new("Import").min_size([74.0, 32.0].into()))
                        .on_hover_text("Import a folder structure")
                        .clicked()
                    {
                        self.open_import();
                    }
                    let selected = !self.selected_items.is_empty();
                    let multi = self.selected_items.len() > 1;
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new(if multi { "Edit all..." } else { "Edit" })
                                .min_size([74.0, 32.0].into()),
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
                        .add_enabled(
                            selected,
                            egui::Button::new("Remove").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Remove selected items (Delete)")
                        .clicked()
                    {
                        self.delete_pending = true;
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Duplicate").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Duplicate selected items (Ctrl+D)")
                        .clicked()
                    {
                        self.duplicate_selected();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Copy").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Copy selected items to clipboard (Ctrl+C)")
                        .clicked()
                    {
                        self.copy_selected();
                    }
                    if ui
                        .add_enabled(
                            !self.clipboard.is_empty(),
                            egui::Button::new("Paste").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Paste clipboard items here (Ctrl+V)")
                        .clicked()
                    {
                        self.paste_clipboard();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Move").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Move up/down or to another menu")
                        .clicked()
                    {
                        self.move_pending = true;
                    }
                });
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let button_widths = 120.0 + 90.0 + 90.0 + ui.spacing().item_spacing.x * 2.0;
                ui.add_space(((ui.available_width() - button_widths) / 2.0).max(0.0));
                if ui
                    .add_enabled(
                        self.load_error.is_none(),
                        egui::Button::new("Save & Close").min_size([120.0, 34.0].into()),
                    )
                    .clicked()
                    && self.save()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui
                    .add_enabled(
                        self.load_error.is_none(),
                        egui::Button::new("Save").min_size([90.0, 34.0].into()),
                    )
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add(egui::Button::new("Close").min_size([90.0, 34.0].into()))
                    .clicked()
                {
                    self.request_close(&ctx);
                }
            });
            ui.horizontal_centered(|ui| {
                if self.dirty {
                    ui.weak("Unsaved changes");
                } else if let Some(status) = &self.status {
                    ui.label(status);
                }
            });

            if let Some(error) = &self.load_error {
                ui.colored_label(
                    egui::Color32::RED,
                    format!("Configuration could not be loaded: {error}"),
                );
                ui.label("The file was not overwritten. Close this window and repair config.json.");
            }
        });
    }

    /// 項目一覧の各行を描く。行全体がドラッグ元になる (FR-6.3) 。
    ///
    /// `Grid` ではなく行ごとの `horizontal` にしているのは、
    /// 行全体を 1 つのドラッグ元として掴めるようにするため。
    pub(super) fn show_item_rows(&mut self, ui: &mut egui::Ui, rows: &[Item]) {
        ui.horizontal(|ui| {
            ui.add_space(ROW_INDENT);
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
                egui::vec2(COL_OPEN, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.strong("Open"),
            );
            ui.strong("Location or content");
        });
        ui.separator();

        // フォーカスは毎回描画から拾い直す
        self.focused_row = None;

        // ドラッグ中に「何番目の手前へ入るか」。行の上半分/下半分で決める
        let mut insert_at: Option<usize> = None;
        let mut dropped: Option<(usize, usize)> = None;

        for (index, item) in rows.iter().enumerate() {
            let id = egui::Id::new(("item_row", &self.selected_menu, index));
            let response = ui
                .dnd_drag_source(id, DragRow { index }, |ui| {
                    self.show_item_row(ui, index, item, id);
                })
                .response;

            // 行全体 (ドラッグ元) をキーボードフォーカスの単位にする。
            // `dnd_drag_source` は `Sense::drag()` で掴むが、これは
            // `FOCUSABLE` を含むのでフォーカス候補になる。矢印キーは
            // この矩形を基準に隣の行を探す
            if response.has_focus() {
                self.focused_row = Some(index);
            }
            if self.focus_selected_row && self.active_item == Some(index) {
                response.request_focus();
                self.focus_selected_row = false;
            }

            // 掴んでいる行の上半分なら手前、下半分なら後ろへ挿入する
            if let Some(pointer) = ui.ctx().pointer_interact_pos()
                && response.dnd_hover_payload::<DragRow>().is_some()
            {
                let rect = response.rect;
                let after = pointer.y > rect.center().y;
                let at = if after { index + 1 } else { index };
                insert_at = Some(at);
                draw_insert_line(ui, rect, after);
            }

            if let Some(payload) = response.dnd_release_payload::<DragRow>() {
                let at = insert_at.unwrap_or(index);
                dropped = Some((payload.index, at));
            }
        }

        // 一覧の末尾へ落とせるように、残り領域も受け口にする
        let tail = ui.allocate_response(
            egui::vec2(
                ui.available_width(),
                TAIL_DROP_HEIGHT.max(ui.available_height()),
            ),
            egui::Sense::hover(),
        );
        if tail.dnd_hover_payload::<DragRow>().is_some() {
            draw_insert_line(ui, tail.rect, false);
        }
        if let Some(payload) = tail.dnd_release_payload::<DragRow>() {
            dropped = Some((payload.index, rows.len()));
        }

        // 矢印キーは egui のフォーカスを行から行へ動かす。選択を追従させないと
        // 網掛けも右側のボタン (Edit / Remove など) も付いてこない。
        //
        // `gained_focus()` は使えない。egui は矢印での移動を `end_pass` で
        // 差し替えるため、次の描画では「前フレームから持っていた」扱いになり
        // 一度も立たない (`memory/mod.rs` の `end_pass`) 。自前で変化を見る
        if let Some(row) = self.focused_row
            && self.focused_row != self.previous_focused_row
        {
            if self.suppress_focus_select {
                self.suppress_focus_select = false;
            } else {
                self.select_single(row);
            }
        }
        self.previous_focused_row = self.focused_row;

        if let Some((from, at)) = dropped {
            self.reorder(from, at);
        }
    }

    /// 1 行分のセルを描く。列幅は見出しと揃える。
    ///
    /// `row_id` は行 (ドラッグ元) の ID。名前セルもフォーカスを取れてしまうため、
    /// フォーカスが来たら行へ渡し直す。
    pub(super) fn show_item_row(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        item: &Item,
        row_id: egui::Id,
    ) {
        ui.horizontal(|ui| {
            // 掴む場所が分かるようにグリップを置く
            ui.add_space(2.0);
            ui.weak("⠿");

            let selected = self.selected_items.contains(&index);
            let response = ui.allocate_ui_with_layout(
                egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.selectable_label(selected, item.label().unwrap_or("----------------")),
            );
            let label = response.inner;
            // 名前セルもフォーカスを取れる。行と 2 つ並ぶと矢印キーが行の中で
            // 足踏みし、そのまま一覧の外のボタンへ抜けてしまう。
            // フォーカスは行へ寄せて、1 行 1 つに保つ
            if label.has_focus() {
                ui.memory_mut(|memory| memory.request_focus(row_id));
                ui.ctx().request_repaint();
            }
            if label.clicked() {
                // Ctrl/Shift 修飾でトグル選択・範囲選択を切り替える (FR-6)
                let modifiers = ui.ctx().input(|input| input.modifiers);
                if modifiers.command {
                    self.toggle_selection(index);
                } else if modifiers.shift {
                    self.extend_selection_to(index);
                } else {
                    self.select_single(index);
                }
                // クリックによる選択はここで確定済みなので、後続のフォーカス変化
                // 検出 (矢印キー用) がこの行を単一選択で上書きしないようにする
                self.suppress_focus_select = true;
            }
            if label.double_clicked() {
                self.select_single(index);
                self.begin_edit();
            }

            ui.allocate_ui_with_layout(
                egui::vec2(COL_KIND, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(item_kind(item)),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(COL_OPEN, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(item_open(item)),
            );
            ui.label(item_detail(item));
        });
    }
}
