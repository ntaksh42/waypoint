//! メニュー階層の探索・項目の表示用ラベル・インポートプレビューの描画。

use waypoint::config::{Config, Item, OpenMode};

pub(super) fn items_at<'a>(config: &'a Config, path: &[usize]) -> Option<&'a Vec<Item>> {
    let mut items = &config.items;
    for &index in path {
        match items.get(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

pub(super) fn items_at_mut<'a>(
    config: &'a mut Config,
    path: &[usize],
) -> Option<&'a mut Vec<Item>> {
    let mut items = &mut config.items;
    for &index in path {
        match items.get_mut(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

pub(super) fn menu_choices(config: &Config) -> Vec<(Vec<usize>, String)> {
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
            ..
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

pub(super) fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::Folder { .. } => "Folder",
        Item::File { .. } => "File",
        Item::SpecialFolder { .. } => "Special folder",
        Item::Shell { .. } => "Shell location",
        Item::Submenu { .. } => "Submenu",
        Item::Separator { .. } => "Separator",
    }
}

pub(super) fn item_detail(item: &Item) -> &str {
    match item {
        Item::Folder { path, .. } | Item::File { path, .. } => path,
        Item::SpecialFolder { known_folder, .. } => known_folder,
        Item::Shell { target, .. } => target,
        Item::Submenu { .. } | Item::Separator { .. } => "",
    }
}

pub(super) fn item_open(item: &Item) -> &'static str {
    match item {
        Item::Folder { open, .. } | Item::SpecialFolder { open, .. } => {
            match open.unwrap_or_default() {
                OpenMode::NewWindow => "New window",
                OpenMode::Reuse => "Reuse",
            }
        }
        Item::File { .. } | Item::Shell { .. } | Item::Submenu { .. } | Item::Separator { .. } => {
            "—"
        }
    }
}

pub(super) fn show_open_mode(ui: &mut eframe::egui::Ui, mode: &mut OpenMode) {
    ui.label("Open");
    ui.horizontal(|ui| {
        ui.radio_value(mode, OpenMode::NewWindow, "New window");
        ui.radio_value(mode, OpenMode::Reuse, "Reuse Explorer window");
    });
}

pub(super) fn show_import_node(
    ui: &mut eframe::egui::Ui,
    node: &mut waypoint::folder_import::FolderNode,
    depth: usize,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        ui.checkbox(&mut node.included, "");
        ui.add(eframe::egui::TextEdit::singleline(&mut node.name).desired_width(360.0));
    });
    ui.add_enabled_ui(node.included, |ui| {
        for child in &mut node.children {
            show_import_node(ui, child, depth + 1);
        }
    });
}
