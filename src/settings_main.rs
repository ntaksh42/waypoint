//! waypoint 設定エディター (FR-6)。常駐部とは別プロセスで必要時のみ起動する。

#![windows_subsystem = "windows"]

use eframe::egui;
use waypoint::config::{Config, Item, LoadOutcome, OpenMode};

fn main() -> eframe::Result<()> {
    // 設定画面も GUI サブシステム。panic を握り潰さずログへ残す
    waypoint::panic_log::install();

    let add_special_folder = std::env::args().any(|arg| arg == "--add-special-folder");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 560.0])
            .with_min_inner_size([640.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Customize - Waypoint",
        options,
        Box::new(move |creation| {
            creation.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = SettingsApp::load();
            if add_special_folder {
                app.begin_add(DraftKind::SpecialFolder);
            }
            Ok(Box::new(app))
        }),
    )
}

/// 項目一覧の列幅。見出しと各行で共有する。
const COL_NAME: f32 = 170.0;
const COL_KIND: f32 = 96.0;
const COL_OPEN: f32 = 82.0;
/// 見出し行のグリップ分の字下げ。
const ROW_INDENT: f32 = 20.0;
/// 一覧末尾に置く受け口の高さ。ここへ落とすと最後尾へ移る。
const TAIL_DROP_HEIGHT: f32 = 24.0;

/// ドラッグ中の行。掴んだ時点の添字だけを運ぶ。
#[derive(Debug, Clone, Copy)]
struct DragRow {
    index: usize,
}

/// 挿入位置を示す線を引く。`after` なら行の下端、そうでなければ上端。
fn draw_insert_line(ui: &egui::Ui, rect: egui::Rect, after: bool) {
    let y = if after { rect.bottom() } else { rect.top() };
    let stroke = egui::Stroke::new(2.0, ui.visuals().selection.bg_fill);
    ui.painter().hline(rect.x_range(), y, stroke);
}

/// 「元の並びで `insert_at` の手前」を、取り除いた後の添字へ直す。
///
/// 先に `remove(from)` すると `from` より後ろが 1 つ詰まるため、
/// 挿入位置がそれより後ろなら 1 引く。
fn reorder_target(from: usize, insert_at: usize) -> usize {
    if insert_at > from {
        insert_at - 1
    } else {
        insert_at
    }
}

struct SettingsApp {
    config: Config,
    selected_menu: Vec<usize>,
    selected_item: Option<usize>,
    draft: Option<ItemDraft>,
    variables_draft: Option<VariablesDraft>,
    trigger_draft: Option<TriggerDraft>,
    import_draft: Option<ImportDraft>,
    add_pending: bool,
    move_pending: bool,
    delete_pending: bool,
    close_pending: bool,
    dirty: bool,
    load_error: Option<String>,
    status: Option<String>,
}

impl SettingsApp {
    fn load() -> Self {
        let (config, load_error) = match waypoint::config::load() {
            LoadOutcome::Loaded(config) | LoadOutcome::Created(config) => (config, None),
            LoadOutcome::Failed(error) => (Config::default(), Some(error)),
        };
        let selected_item = (!config.items.is_empty()).then_some(0);
        Self {
            config,
            selected_menu: Vec::new(),
            selected_item,
            draft: None,
            variables_draft: None,
            trigger_draft: None,
            import_draft: None,
            add_pending: false,
            move_pending: false,
            delete_pending: false,
            close_pending: false,
            dirty: false,
            load_error,
            status: None,
        }
    }

    fn current_items(&self) -> Option<&Vec<Item>> {
        items_at(&self.config, &self.selected_menu)
    }

    fn current_items_mut(&mut self) -> Option<&mut Vec<Item>> {
        items_at_mut(&mut self.config, &self.selected_menu)
    }

    fn begin_add(&mut self, kind: DraftKind) {
        self.draft = Some(ItemDraft::new(kind));
    }

    fn begin_edit(&mut self) {
        let Some(index) = self.selected_item else {
            return;
        };
        if let Some(item) = self.current_items().and_then(|items| items.get(index)) {
            self.draft = Some(ItemDraft::from_item(index, item));
        }
    }

    fn apply_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let editing = draft.editing;
        let item = draft.into_item();
        let Some(items) = self.current_items_mut() else {
            return;
        };

        if let Some(index) = editing {
            if let Some(slot) = items.get_mut(index) {
                *slot = item;
                self.selected_item = Some(index);
            }
        } else {
            items.push(item);
            self.selected_item = Some(items.len() - 1);
        }
        self.dirty = true;
        self.status = None;
    }

    fn duplicate_selected(&mut self) {
        let Some(index) = self.selected_item else {
            return;
        };
        let Some(item) = self
            .current_items()
            .and_then(|items| items.get(index))
            .cloned()
        else {
            return;
        };
        if let Some(items) = self.current_items_mut() {
            items.insert(index + 1, item);
            self.selected_item = Some(index + 1);
            self.dirty = true;
            self.status = None;
        }
    }

    fn remove_selected(&mut self) {
        let Some(index) = self.selected_item else {
            return;
        };
        if let Some(items) = self.current_items_mut()
            && index < items.len()
        {
            items.remove(index);
            self.selected_item = index.checked_sub(1).or((!items.is_empty()).then_some(0));
            self.dirty = true;
            self.status = None;
        }
    }

    fn move_selected(&mut self, delta: isize) {
        let Some(index) = self.selected_item else {
            return;
        };
        let Some(target) = index.checked_add_signed(delta) else {
            return;
        };
        if let Some(items) = self.current_items_mut()
            && target < items.len()
        {
            items.swap(index, target);
            self.selected_item = Some(target);
            self.dirty = true;
            self.status = None;
        }
    }

    /// ドラッグした行を挿入位置へ移す (FR-6.3) 。
    ///
    /// `insert_at` は「元の並びで何番目の手前に入れるか」なので、
    /// 取り除く操作で後ろの添字が 1 つ詰まる分を補正する。
    fn reorder(&mut self, from: usize, insert_at: usize) {
        let Some(items) = self.current_items_mut() else {
            return;
        };
        if from >= items.len() || insert_at > items.len() {
            return;
        }
        let target = reorder_target(from, insert_at);
        if target == from {
            return;
        }
        let item = items.remove(from);
        items.insert(target, item);
        self.selected_item = Some(target);
        self.dirty = true;
        self.status = None;
    }

    fn add_dropped_folders(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            if let Some(items) = self.current_items_mut() {
                items.push(Item::Folder {
                    name,
                    path: path.display().to_string(),
                    open: None,
                    icon: None,
                    show_branch: false,
                });
                self.selected_item = Some(items.len() - 1);
                self.dirty = true;
                self.status = None;
            }
        }
    }

    fn save(&mut self) -> bool {
        if self.load_error.is_some() {
            return false;
        }
        match waypoint::config::save(&self.config) {
            Ok(()) => {
                self.dirty = false;
                self.status = Some(if waypoint::tray::signal_reload() {
                    "Saved and reloaded.".to_string()
                } else {
                    "Saved. Waypoint is not running.".to_string()
                });
                true
            }
            Err(error) => {
                self.status = Some(format!("Save failed: {error}"));
                false
            }
        }
    }

    fn open_variables(&mut self) {
        self.variables_draft = Some(VariablesDraft {
            entries: self
                .config
                .variables
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            error: None,
        });
    }

    fn open_trigger(&mut self) {
        self.trigger_draft = Some(TriggerDraft::from_config(&self.config));
    }

    fn open_import(&mut self) {
        self.import_draft = Some(ImportDraft {
            root: String::new(),
            depth: 2,
            preview: None,
            error: None,
        });
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        if self.dirty {
            self.close_pending = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.draft.is_some()
            || self.variables_draft.is_some()
            || self.trigger_draft.is_some()
            || self.import_draft.is_some()
            || self.add_pending
            || self.move_pending
            || self.delete_pending
            || self.close_pending
        {
            return;
        }
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::S,
            ))
        }) {
            self.save();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.begin_edit();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            && self.selected_item.is_some()
        {
            self.delete_pending = true;
        }
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::D,
            ))
        }) {
            self.duplicate_selected();
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp)) {
            self.move_selected(-1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown)) {
            self.move_selected(1);
        }
    }

    fn show_items(&mut self, root: &mut egui::Ui) {
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
                    if ui.button("Add special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
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
                    ui.separator();
                    if ui
                        .add_enabled(self.selected_item.is_some(), egui::Button::new("Edit"))
                        .clicked()
                    {
                        self.begin_edit();
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
                            self.selected_item = items_at(&self.config, &path)
                                .and_then(|items| (!items.is_empty()).then_some(0));
                            self.selected_menu = path;
                        }
                    }
                });
            ui.add_space(8.0);

            let rows = self.current_items().cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let selected = self.selected_item.is_some();
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
                    let selected = self.selected_item.is_some();
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Edit").min_size([74.0, 32.0].into()),
                        )
                        .clicked()
                    {
                        self.begin_edit();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Remove").min_size([74.0, 32.0].into()),
                        )
                        .clicked()
                    {
                        self.delete_pending = true;
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Copy").min_size([74.0, 32.0].into()),
                        )
                        .clicked()
                    {
                        self.duplicate_selected();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Move").min_size([74.0, 32.0].into()),
                        )
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
    fn show_item_rows(&mut self, ui: &mut egui::Ui, rows: &[Item]) {
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

        // ドラッグ中に「何番目の手前へ入るか」。行の上半分/下半分で決める
        let mut insert_at: Option<usize> = None;
        let mut dropped: Option<(usize, usize)> = None;

        for (index, item) in rows.iter().enumerate() {
            let id = egui::Id::new(("item_row", &self.selected_menu, index));
            let response = ui
                .dnd_drag_source(id, DragRow { index }, |ui| {
                    self.show_item_row(ui, index, item);
                })
                .response;

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

        if let Some((from, at)) = dropped {
            self.reorder(from, at);
        }
    }

    /// 1 行分のセルを描く。列幅は見出しと揃える。
    fn show_item_row(&mut self, ui: &mut egui::Ui, index: usize, item: &Item) {
        ui.horizontal(|ui| {
            // 掴む場所が分かるようにグリップを置く
            ui.add_space(2.0);
            ui.weak("⠿");

            let selected = self.selected_item == Some(index);
            let response = ui.allocate_ui_with_layout(
                egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.selectable_label(selected, item.label().unwrap_or("----------------")),
            );
            if response.inner.clicked() {
                self.selected_item = Some(index);
            }
            if response.inner.double_clicked() {
                self.selected_item = Some(index);
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

    fn show_item_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(if draft.editing.is_some() {
            "Edit item"
        } else {
            "Add item"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            match draft.kind {
                DraftKind::Folder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Path");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut draft.path);
                        if ui.button("Browse...").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            if draft.name.is_empty()
                                && let Some(name) = path.file_name()
                            {
                                draft.name = name.to_string_lossy().into_owned();
                            }
                            draft.path = path.display().to_string();
                        }
                    });
                    show_open_mode(ui, &mut draft.open);
                    ui.checkbox(&mut draft.show_branch, "Show Git branch name")
                        .on_hover_text(
                            "Appends [branch] to the menu label when the path is inside a Git work tree.",
                        );
                }
                DraftKind::SpecialFolder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Known folder");
                    egui::ComboBox::from_id_salt("known_folder")
                        .selected_text(&draft.known_folder)
                        .show_ui(ui, |ui| {
                            for name in waypoint::known_folder::NAMES {
                                ui.selectable_value(
                                    &mut draft.known_folder,
                                    (*name).to_string(),
                                    *name,
                                );
                            }
                        });
                    show_open_mode(ui, &mut draft.open);
                }
                DraftKind::Submenu => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                }
                DraftKind::Separator => {
                    ui.label("Heading (optional)");
                    ui.text_edit_singleline(&mut draft.name);
                }
            }

            if let Some(error) = &draft.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                apply = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });

        if apply {
            if let Some(error) = draft.validate() {
                draft.error = Some(error);
            } else {
                self.apply_draft();
            }
        } else if cancel {
            self.draft = None;
        }
    }

    fn show_variables_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.variables_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let mut remove = None;
        egui::Window::new("Variables")
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("variables_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Value");
                        ui.end_row();
                        for (index, (name, value)) in draft.entries.iter_mut().enumerate() {
                            ui.text_edit_singleline(name);
                            ui.text_edit_singleline(value);
                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if ui.button("Add variable").clicked() {
                    draft.entries.push((String::new(), String::new()));
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if let Some(index) = remove {
            draft.entries.remove(index);
        }
        if apply {
            let mut variables = std::collections::BTreeMap::new();
            let mut error = None;
            for (name, value) in &draft.entries {
                let name = name.trim();
                if name.is_empty() {
                    error = Some("Variable name is required.".to_string());
                    break;
                }
                if variables.insert(name.to_string(), value.clone()).is_some() {
                    error = Some(format!("Variable name is duplicated: {name}"));
                    break;
                }
            }
            if let Some(error) = error {
                draft.error = Some(error);
            } else {
                self.config.variables = variables;
                self.variables_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.variables_draft = None;
        }
    }

    fn show_trigger_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.trigger_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Trigger")
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.checkbox(&mut draft.middle_click, "Enable middle click");
                ui.label("Hotkey");
                ui.text_edit_singleline(&mut draft.hotkey);
                ui.weak("Example: Win+W");
                ui.separator();
                ui.strong("Quick Launch");
                ui.label("Hotkey");
                ui.text_edit_singleline(&mut draft.quick_launch_hotkey);
                ui.checkbox(&mut draft.include_recent_folders, "Include Recent Folders");
                ui.checkbox(
                    &mut draft.include_frequent_folders,
                    "Include Frequent Folders",
                );
                ui.checkbox(&mut draft.search_paths, "Search folder paths");
                ui.horizontal(|ui| {
                    ui.label("Visible results");
                    ui.add(egui::DragValue::new(&mut draft.visible_results).range(12..=24));
                });
                ui.separator();
                ui.label("Excluded processes (one per line)");
                ui.add(
                    egui::TextEdit::multiline(&mut draft.excluded_processes)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY),
                );
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if apply {
            if waypoint::trigger::parse_hotkey(draft.hotkey.trim()).is_none() {
                draft.error = Some("Hotkey is invalid.".to_string());
            } else if waypoint::trigger::parse_hotkey(draft.quick_launch_hotkey.trim()).is_none() {
                draft.error = Some("Quick Launch hotkey is invalid.".to_string());
            } else if draft
                .hotkey
                .trim()
                .eq_ignore_ascii_case(draft.quick_launch_hotkey.trim())
            {
                draft.error = Some("The two hotkeys must be different.".to_string());
            } else {
                self.config.settings.trigger.middle_click = draft.middle_click;
                self.config.settings.trigger.hotkey = draft.hotkey.trim().to_string();
                self.config.settings.trigger.excluded_processes = draft
                    .excluded_processes
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                self.config.settings.quick_launch.hotkey =
                    draft.quick_launch_hotkey.trim().to_string();
                self.config.settings.quick_launch.include_recent_folders =
                    draft.include_recent_folders;
                self.config.settings.quick_launch.include_frequent_folders =
                    draft.include_frequent_folders;
                self.config.settings.quick_launch.search_paths = draft.search_paths;
                self.config.settings.quick_launch.visible_results =
                    draft.visible_results.clamp(12, 24);
                self.trigger_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.trigger_draft = None;
        }
    }

    fn show_import_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.import_draft.as_mut() else {
            return;
        };
        let mut refresh = false;
        let mut apply = false;
        let mut cancel = false;

        egui::Window::new("Import folder structure")
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

        if apply {
            let item = self
                .import_draft
                .as_ref()
                .and_then(|draft| draft.preview.as_ref())
                .and_then(waypoint::folder_import::FolderNode::to_item);
            if let Some(item) = item {
                if let Some(items) = self.current_items_mut() {
                    items.push(item);
                    self.selected_item = Some(items.len() - 1);
                    self.dirty = true;
                    self.status = None;
                }
                self.import_draft = None;
            }
        } else if cancel {
            self.import_draft = None;
        }
    }

    fn show_confirmations(&mut self, ctx: &egui::Context) {
        if self.add_pending {
            egui::Window::new("Add favorite")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if ui.button("Folder").clicked() {
                        self.begin_add(DraftKind::Folder);
                        self.add_pending = false;
                    }
                    if ui.button("Special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
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
                    if ui.button("Cancel").clicked() {
                        self.add_pending = false;
                    }
                });
        }

        if self.move_pending {
            egui::Window::new("Move favorite")
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
                        if ui.button("Cancel").clicked() {
                            self.move_pending = false;
                        }
                    });
                });
        }

        if self.delete_pending {
            egui::Window::new("Remove item?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("The selected item will be removed.");
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
        }

        if self.close_pending {
            egui::Window::new("Discard changes?")
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
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_pending = true;
        }
        self.add_dropped_folders(&ctx);
        self.handle_shortcuts(&ctx);
        self.show_items(ui);
        self.show_item_editor(&ctx);
        self.show_variables_editor(&ctx);
        self.show_trigger_editor(&ctx);
        self.show_import_editor(&ctx);
        self.show_confirmations(&ctx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DraftKind {
    Folder,
    SpecialFolder,
    Submenu,
    Separator,
}

struct ItemDraft {
    editing: Option<usize>,
    kind: DraftKind,
    name: String,
    path: String,
    known_folder: String,
    open: OpenMode,
    icon: Option<String>,
    show_branch: bool,
    submenu_items: Vec<Item>,
    error: Option<String>,
}

struct VariablesDraft {
    entries: Vec<(String, String)>,
    error: Option<String>,
}

struct TriggerDraft {
    middle_click: bool,
    hotkey: String,
    excluded_processes: String,
    quick_launch_hotkey: String,
    include_recent_folders: bool,
    include_frequent_folders: bool,
    search_paths: bool,
    visible_results: usize,
    error: Option<String>,
}

struct ImportDraft {
    root: String,
    depth: usize,
    preview: Option<waypoint::folder_import::FolderNode>,
    error: Option<String>,
}

impl TriggerDraft {
    fn from_config(config: &Config) -> Self {
        let trigger = &config.settings.trigger;
        let quick_launch = &config.settings.quick_launch;
        Self {
            middle_click: trigger.middle_click,
            hotkey: trigger.hotkey.clone(),
            excluded_processes: trigger.excluded_processes.join("\n"),
            quick_launch_hotkey: quick_launch.hotkey.clone(),
            include_recent_folders: quick_launch.include_recent_folders,
            include_frequent_folders: quick_launch.include_frequent_folders,
            search_paths: quick_launch.search_paths,
            visible_results: quick_launch.visible_results,
            error: None,
        }
    }
}

impl ItemDraft {
    fn new(kind: DraftKind) -> Self {
        Self {
            editing: None,
            kind,
            name: String::new(),
            path: String::new(),
            known_folder: waypoint::known_folder::NAMES[0].to_string(),
            open: OpenMode::default(),
            icon: None,
            show_branch: false,
            submenu_items: Vec::new(),
            error: None,
        }
    }

    fn from_item(index: usize, item: &Item) -> Self {
        let mut draft = match item {
            Item::Folder {
                name,
                path,
                open,
                icon,
                show_branch,
            } => Self {
                kind: DraftKind::Folder,
                name: name.clone(),
                path: path.clone(),
                open: open.unwrap_or_default(),
                icon: icon.clone(),
                show_branch: *show_branch,
                ..Self::new(DraftKind::Folder)
            },
            Item::SpecialFolder {
                name,
                known_folder,
                open,
            } => Self {
                kind: DraftKind::SpecialFolder,
                name: name.clone(),
                known_folder: known_folder.clone(),
                open: open.unwrap_or_default(),
                ..Self::new(DraftKind::SpecialFolder)
            },
            Item::Submenu { name, items } => Self {
                kind: DraftKind::Submenu,
                name: name.clone(),
                submenu_items: items.clone(),
                ..Self::new(DraftKind::Submenu)
            },
            Item::Separator { name } => Self {
                kind: DraftKind::Separator,
                name: name.clone().unwrap_or_default(),
                ..Self::new(DraftKind::Separator)
            },
        };
        draft.editing = Some(index);
        draft
    }

    fn validate(&self) -> Option<String> {
        match self.kind {
            DraftKind::Folder if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            DraftKind::Folder if self.path.trim().is_empty() => {
                Some("Path is required.".to_string())
            }
            DraftKind::SpecialFolder | DraftKind::Submenu if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            _ => None,
        }
    }

    fn into_item(self) -> Item {
        let open = (self.open != OpenMode::default()).then_some(self.open);
        match self.kind {
            DraftKind::Folder => Item::Folder {
                name: self.name,
                path: self.path,
                open,
                icon: self.icon,
                show_branch: self.show_branch,
            },
            DraftKind::SpecialFolder => Item::SpecialFolder {
                name: self.name,
                known_folder: self.known_folder,
                open,
            },
            DraftKind::Submenu => Item::Submenu {
                name: self.name,
                items: self.submenu_items,
            },
            DraftKind::Separator => Item::Separator {
                name: (!self.name.trim().is_empty()).then_some(self.name),
            },
        }
    }
}

fn items_at<'a>(config: &'a Config, path: &[usize]) -> Option<&'a Vec<Item>> {
    let mut items = &config.items;
    for &index in path {
        match items.get(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

fn items_at_mut<'a>(config: &'a mut Config, path: &[usize]) -> Option<&'a mut Vec<Item>> {
    let mut items = &mut config.items;
    for &index in path {
        match items.get_mut(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

fn menu_choices(config: &Config) -> Vec<(Vec<usize>, String)> {
    let mut choices = vec![(Vec::new(), "Main".to_string())];
    collect_menu_choices(&config.items, &mut Vec::new(), "", &mut choices);
    choices
}

fn collect_menu_choices(
    items: &[Item],
    path: &mut Vec<usize>,
    parent_name: &str,
    choices: &mut Vec<(Vec<usize>, String)>,
) {
    for (index, item) in items.iter().enumerate() {
        let Item::Submenu {
            name,
            items: children,
        } = item
        else {
            continue;
        };
        path.push(index);
        let full_name = if parent_name.is_empty() {
            name.clone()
        } else {
            format!("{parent_name} > {name}")
        };
        choices.push((path.clone(), full_name.clone()));
        collect_menu_choices(children, path, &full_name, choices);
        path.pop();
    }
}

fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::Folder { .. } => "Folder",
        Item::SpecialFolder { .. } => "Special folder",
        Item::Submenu { .. } => "Submenu",
        Item::Separator { .. } => "Separator",
    }
}

fn item_detail(item: &Item) -> &str {
    match item {
        Item::Folder { path, .. } => path,
        Item::SpecialFolder { known_folder, .. } => known_folder,
        Item::Submenu { .. } | Item::Separator { .. } => "",
    }
}

fn item_open(item: &Item) -> &'static str {
    match item {
        Item::Folder { open, .. } | Item::SpecialFolder { open, .. } => {
            match open.unwrap_or_default() {
                OpenMode::NewWindow => "New window",
                OpenMode::Reuse => "Reuse",
            }
        }
        Item::Submenu { .. } | Item::Separator { .. } => "—",
    }
}

fn show_open_mode(ui: &mut egui::Ui, mode: &mut OpenMode) {
    ui.label("Open");
    ui.horizontal(|ui| {
        ui.radio_value(mode, OpenMode::NewWindow, "New window");
        ui.radio_value(mode, OpenMode::Reuse, "Reuse Explorer window");
    });
}

fn show_import_node(
    ui: &mut egui::Ui,
    node: &mut waypoint::folder_import::FolderNode,
    depth: usize,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        ui.checkbox(&mut node.included, "");
        ui.add(egui::TextEdit::singleline(&mut node.name).desired_width(360.0));
    });
    ui.add_enabled_ui(node.included, |ui| {
        for child in &mut node.children {
            show_import_node(ui, child, depth + 1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_item_path_resolves_submenu() {
        let mut config = Config {
            items: vec![Item::Submenu {
                name: "Tools".to_string(),
                items: vec![Item::Separator { name: None }],
            }],
            ..Default::default()
        };

        items_at_mut(&mut config, &[0])
            .unwrap()
            .push(Item::Separator {
                name: Some("Build".to_string()),
            });

        assert_eq!(items_at(&config, &[0]).unwrap().len(), 2);
    }

    #[test]
    fn editing_submenu_preserves_its_children() {
        let original = Item::Submenu {
            name: "Old".to_string(),
            items: vec![Item::Separator { name: None }],
        };
        let mut draft = ItemDraft::from_item(0, &original);
        draft.name = "New".to_string();

        assert_eq!(
            draft.into_item(),
            Item::Submenu {
                name: "New".to_string(),
                items: vec![Item::Separator { name: None }],
            }
        );
    }

    /// 実際に remove → insert したときの並びを返す。
    /// `reorder_target` の補正が正しいことを、結果の並びで確かめる。
    fn reordered(len: usize, from: usize, insert_at: usize) -> Vec<usize> {
        let mut items: Vec<usize> = (0..len).collect();
        let item = items.remove(from);
        items.insert(reorder_target(from, insert_at), item);
        items
    }

    /// 後ろへ動かすとき、取り除きで添字が 1 つ詰まる分を補正する。
    #[test]
    fn dragging_down_lands_before_the_target() {
        // [0,1,2,3,4] の 1 を「3 の手前」へ → [0,2,1,3,4]
        assert_eq!(reordered(5, 1, 3), vec![0, 2, 1, 3, 4]);
    }

    /// 前へ動かすときは補正しない。
    #[test]
    fn dragging_up_lands_at_the_target() {
        // [0,1,2,3,4] の 3 を「1 の手前」へ → [0,3,1,2,4]
        assert_eq!(reordered(5, 3, 1), vec![0, 3, 1, 2, 4]);
    }

    /// 一覧の末尾へ落とすと最後尾に来る。
    #[test]
    fn dragging_to_the_tail_lands_last() {
        assert_eq!(reordered(4, 0, 4), vec![1, 2, 3, 0]);
    }

    /// 自分の位置とその直後は、どちらも並びを変えない。
    #[test]
    fn dropping_onto_itself_keeps_the_order() {
        assert_eq!(reordered(4, 2, 2), vec![0, 1, 2, 3]);
        assert_eq!(reordered(4, 2, 3), vec![0, 1, 2, 3]);
    }
}
