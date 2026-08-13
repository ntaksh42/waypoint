//! waypoint 設定エディター (FR-6)。常駐部とは別プロセスで必要時のみ起動する。

#![windows_subsystem = "windows"]

use eframe::egui;
use waypoint::config::{Config, Item, LoadOutcome, OpenMode};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 600.0])
            .with_min_inner_size([760.0, 440.0]),
        ..Default::default()
    };

    eframe::run_native(
        "waypoint settings",
        options,
        Box::new(|_| Ok(Box::new(SettingsApp::load()))),
    )
}

struct SettingsApp {
    config: Config,
    selected_menu: Vec<usize>,
    selected_item: Option<usize>,
    draft: Option<ItemDraft>,
    variables_draft: Option<VariablesDraft>,
    trigger_draft: Option<TriggerDraft>,
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
        Self {
            config,
            selected_menu: Vec::new(),
            selected_item: None,
            draft: None,
            variables_draft: None,
            trigger_draft: None,
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
                });
                self.selected_item = Some(items.len() - 1);
                self.dirty = true;
                self.status = None;
            }
        }
    }

    fn save(&mut self) {
        if self.load_error.is_some() {
            return;
        }
        match waypoint::config::save(&self.config) {
            Ok(()) => {
                self.dirty = false;
                self.status = Some(if waypoint::tray::signal_reload() {
                    "Saved and reloaded.".to_string()
                } else {
                    "Saved. Waypoint is not running.".to_string()
                });
            }
            Err(error) => self.status = Some(format!("Save failed: {error}")),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.draft.is_some()
            || self.variables_draft.is_some()
            || self.trigger_draft.is_some()
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

    fn show_menu_tree(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("menus")
            .default_size(220.0)
            .resizable(true)
            .show(root, |ui| {
                ui.heading("Menus");
                ui.separator();
                if ui
                    .selectable_label(self.selected_menu.is_empty(), "Root")
                    .clicked()
                {
                    self.selected_menu.clear();
                    self.selected_item = None;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut path = Vec::new();
                    show_submenus(
                        ui,
                        &self.config.items,
                        &mut path,
                        &mut self.selected_menu,
                        &mut self.selected_item,
                    );
                });
            });
    }

    fn show_items(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Items");
                ui.weak("Drop folders here to add them");
            });
            ui.separator();

            let rows = self.current_items().cloned().unwrap_or_default();
            egui::ScrollArea::both().show(ui, |ui| {
                egui::Grid::new("items_grid")
                    .striped(true)
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Type");
                        ui.strong("Path / target");
                        ui.end_row();

                        for (index, item) in rows.iter().enumerate() {
                            let response = ui.selectable_label(
                                self.selected_item == Some(index),
                                item.label().unwrap_or("—"),
                            );
                            if response.clicked() {
                                self.selected_item = Some(index);
                            }
                            if response.double_clicked() {
                                self.selected_item = Some(index);
                                self.begin_edit();
                            }
                            ui.label(item_kind(item));
                            ui.label(item_detail(item));
                            ui.end_row();
                        }
                    });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Add folder").clicked() {
                    self.begin_add(DraftKind::Folder);
                }
                if ui.button("Add special folder").clicked() {
                    self.begin_add(DraftKind::SpecialFolder);
                }
                if ui.button("Add submenu").clicked() {
                    self.begin_add(DraftKind::Submenu);
                }
                if ui.button("Add separator").clicked() {
                    self.begin_add(DraftKind::Separator);
                }
                ui.separator();
                let selected = self.selected_item.is_some();
                if ui
                    .add_enabled(selected, egui::Button::new("Edit"))
                    .clicked()
                {
                    self.begin_edit();
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Duplicate"))
                    .clicked()
                {
                    self.duplicate_selected();
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Remove"))
                    .clicked()
                {
                    self.delete_pending = true;
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Move up"))
                    .clicked()
                {
                    self.move_selected(-1);
                }
                if ui
                    .add_enabled(selected, egui::Button::new("Move down"))
                    .clicked()
                {
                    self.move_selected(1);
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Variables...").clicked() {
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
                if ui.button("Trigger...").clicked() {
                    self.trigger_draft = Some(TriggerDraft::from_config(&self.config));
                }
                ui.separator();
                if ui
                    .add_enabled(self.load_error.is_none(), egui::Button::new("Save"))
                    .clicked()
                {
                    self.save();
                }
                if ui.button("Close").clicked() {
                    if self.dirty {
                        self.close_pending = true;
                    } else {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                if self.dirty {
                    ui.weak("Unsaved changes");
                }
                if let Some(status) = &self.status {
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
                ui.weak("Example: Ctrl+Alt+W");
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
                self.trigger_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.trigger_draft = None;
        }
    }

    fn show_confirmations(&mut self, ctx: &egui::Context) {
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
        self.show_menu_tree(ui);
        self.show_items(ui);
        self.show_item_editor(&ctx);
        self.show_variables_editor(&ctx);
        self.show_trigger_editor(&ctx);
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
    error: Option<String>,
}

impl TriggerDraft {
    fn from_config(config: &Config) -> Self {
        let trigger = &config.settings.trigger;
        Self {
            middle_click: trigger.middle_click,
            hotkey: trigger.hotkey.clone(),
            excluded_processes: trigger.excluded_processes.join("\n"),
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
            } => Self {
                kind: DraftKind::Folder,
                name: name.clone(),
                path: path.clone(),
                open: open.unwrap_or_default(),
                icon: icon.clone(),
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

fn show_submenus(
    ui: &mut egui::Ui,
    items: &[Item],
    path: &mut Vec<usize>,
    selected_menu: &mut Vec<usize>,
    selected_item: &mut Option<usize>,
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
        let current_path = path.clone();
        let response = egui::CollapsingHeader::new(name)
            .id_salt(&current_path)
            .default_open(true)
            .show(ui, |ui| {
                show_submenus(ui, children, path, selected_menu, selected_item);
            });
        if response.header_response.clicked() {
            *selected_menu = current_path;
            *selected_item = None;
        }
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

fn show_open_mode(ui: &mut egui::Ui, mode: &mut OpenMode) {
    ui.label("Open");
    ui.horizontal(|ui| {
        ui.radio_value(mode, OpenMode::NewWindow, "New window");
        ui.radio_value(mode, OpenMode::Reuse, "Reuse Explorer window");
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
}
