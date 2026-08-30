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

/// 全メニュー横断の項目検索 1 件分。
pub(super) struct SearchHit {
    /// ヒットした項目が属するメニューへのパス。
    pub(super) menu_path: Vec<usize>,
    /// そのメニューの表示名 ("Main" または "Parent > Child")。
    pub(super) menu_name: String,
    /// メニュー内での添字。
    pub(super) index: usize,
    pub(super) kind: &'static str,
    pub(super) label: String,
    pub(super) detail: String,
}

/// 名前またはパス/ターゲットに `query` を含む項目を、全メニューを再帰的に
/// たどって集める (FR-6.14)。プロジェクト数が多い環境で、どのサブメニューに
/// 入れたか覚えていなくても項目を見つけられるようにするための横断検索。
pub(super) fn search_items(config: &Config, query: &str) -> Vec<SearchHit> {
    let query = query.trim().to_lowercase();
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    search_items_rec(&config.items, &mut Vec::new(), "", &query, &mut hits);
    hits
}

fn search_items_rec(
    items: &[Item],
    path: &mut Vec<usize>,
    parent_name: &str,
    query: &str,
    hits: &mut Vec<SearchHit>,
) {
    for (index, item) in items.iter().enumerate() {
        if let Some(label) = item.label() {
            let detail = item_detail(item);
            if label.to_lowercase().contains(query) || detail.to_lowercase().contains(query) {
                hits.push(SearchHit {
                    menu_path: path.clone(),
                    menu_name: if parent_name.is_empty() {
                        "Main".to_string()
                    } else {
                        parent_name.to_string()
                    },
                    index,
                    kind: item_kind(item),
                    label: label.to_string(),
                    detail: detail.to_string(),
                });
            }
        }
        if let Item::Submenu {
            name,
            items: children,
            ..
        } = item
        {
            path.push(index);
            let full_name = if parent_name.is_empty() {
                name.clone()
            } else {
                format!("{parent_name} > {name}")
            };
            search_items_rec(children, path, &full_name, query, hits);
            path.pop();
        }
    }
}

/// メニュー選択肢を `query` で絞り込む。一致したメニューと、そこへ至る祖先を
/// 残し、それ以外は隠す (FR-6.15)。階層を保ったまま辿れるようにするため、
/// 一致自体だけを残すのではなく祖先も一緒に返す。
pub(super) fn filter_menu_choices<'a>(
    choices: &'a [(Vec<usize>, String)],
    query: &str,
) -> Vec<&'a (Vec<usize>, String)> {
    let query = query.trim();
    if query.is_empty() {
        return choices.iter().collect();
    }
    let query = query.to_lowercase();
    let matching_paths: Vec<&Vec<usize>> = choices
        .iter()
        .filter(|(_, name)| name.to_lowercase().contains(&query))
        .map(|(path, _)| path)
        .collect();
    choices
        .iter()
        .filter(|(path, _)| {
            matching_paths
                .iter()
                .any(|m| m.len() >= path.len() && m[..path.len()] == path[..])
        })
        .collect()
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
