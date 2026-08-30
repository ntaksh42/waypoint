mod azure_test;
mod edit_test;
mod layout_test;
mod search_test;
mod selection_test;

use waypoint::config::{Config, Item};

use super::app::SettingsApp;

/// テスト用に `n` 個の Folder 項目を持つ最小の SettingsApp を組み立てる。
pub(super) fn app_with_folders(n: usize) -> SettingsApp {
    let items = (0..n)
        .map(|i| Item::Folder {
            name: format!("f{i}"),
            path: format!("C:/f{i}"),
            open: None,
            icon: None,
            show_branch: false,
        })
        .collect();
    SettingsApp {
        config: Config {
            items,
            ..Default::default()
        },
        selected_menu: Vec::new(),
        search: String::new(),
        menu_filter: String::new(),
        selected_items: std::collections::BTreeSet::new(),
        active_item: None,
        selection_anchor: None,
        draft: None,
        batch_draft: None,
        move_to_menu_draft: None,
        variables_draft: None,
        trigger_draft: None,
        azure_project_picker: None,
        import_draft: None,
        clipboard: Vec::new(),
        delete_pending: false,
        close_pending: false,
        dirty: false,
        load_error: None,
        status: None,
        focused_row: None,
        previous_focused_row: None,
        focus_selected_row: false,
        suppress_focus_select: false,
    }
}

pub(super) fn names(app: &SettingsApp) -> Vec<String> {
    app.config
        .items
        .iter()
        .map(|item| item.label().unwrap_or_default().to_string())
        .collect()
}
