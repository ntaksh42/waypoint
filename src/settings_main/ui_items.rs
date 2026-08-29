//! 項目一覧の描画: 一覧テーブルと各行のドラッグ&ドロップ。
//! ツールバーは `ui_toolbar`、左のメニューツリーは `ui_tree` に分離している。

use eframe::egui;
use waypoint::config::Item;

use super::app::SettingsApp;
use super::helpers::{item_detail, item_kind, item_open};
use super::{COL_KIND, COL_NAME, COL_OPEN, ROW_INDENT, TAIL_DROP_HEIGHT};

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
    /// エクスプローラ型レイアウト: 左にメニューツリー、右に選択中メニューの項目一覧。
    /// 上部にツールバー、下部に Save/Close と状態表示を置く。
    pub(super) fn show_items(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::Panel::top("toolbar").show(root, |ui| {
            ui.add_space(4.0);
            self.show_toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("footer").show(root, |ui| {
            ui.add_space(6.0);
            if let Some(error) = &self.load_error {
                ui.colored_label(
                    egui::Color32::RED,
                    format!("Configuration could not be loaded: {error}"),
                );
                ui.label("The file was not overwritten. Close this window and repair config.json.");
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.load_error.is_none(), egui::Button::new("Save & Close"))
                    .clicked()
                    && self.save()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui
                    .add_enabled(self.load_error.is_none(), egui::Button::new("Save"))
                    .clicked()
                {
                    self.save();
                }
                if ui.button("Close").clicked() {
                    self.request_close(&ctx);
                }
                if self.dirty {
                    ui.weak("Unsaved changes");
                } else if let Some(status) = &self.status {
                    ui.label(status);
                }
            });
            ui.add_space(6.0);
        });
        egui::Panel::left("menu_tree_panel")
            .resizable(true)
            .default_size(200.0)
            .size_range(140.0..=360.0)
            .show(root, |ui| {
                self.show_menu_tree(ui);
            });
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .id(super::search_box_id())
                        .hint_text("name or path, across all menus (Ctrl+F)")
                        .desired_width(320.0),
                );
                if !self.search.is_empty() && ui.small_button("Clear (Esc)").clicked() {
                    self.search.clear();
                }
            });
            if self.search.trim().is_empty() {
                ui.weak(self.current_menu_breadcrumb());
            }
            ui.add_space(2.0);

            // `horizontal` は中身の自然な高さに縮む。パネルの残り高さを
            // 明示しないと、一覧が数行ぶんで打ち切られ、下に使われない
            // 余白が残ったまま項目が見切れる (ScrollArea の
            // `auto_shrink` では直らない。制約はこの外側で決まるため)。
            // 検索ボックス・パンくずを描いた後に測ることで、それらの分は
            // 自動的に差し引かれる。
            let body_height = ui.available_height();
            if !self.search.trim().is_empty() {
                self.show_search_results(ui, body_height);
                return;
            }
            ui.horizontal(|ui| {
                ui.set_height(body_height);
                let selected = !self.selected_items.is_empty();
                ui.vertical(|ui| {
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

                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    let rows = self.current_items().cloned().unwrap_or_default();
                    // 下の説明文ぶんを先に確保してから、残りを一覧へ回す。
                    // 引かないと枠が説明文を押し出して見切れる。
                    let hint_height = ui.text_style_height(&egui::TextStyle::Body)
                        + ui.spacing().item_spacing.y * 2.0;
                    let list_height = (body_height - hint_height).max(80.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::both()
                            .max_height(list_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                self.show_item_rows(ui, &rows);
                            });
                    });
                    ui.weak("Drag rows to reorder. Drop folders into the list to add them");
                });
            });
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
