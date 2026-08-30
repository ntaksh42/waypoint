use waypoint::config::{Config, Item};

use super::super::helpers::{filter_menu_choices, menu_choices, search_items};
use super::app_with_folders;

fn folder(name: &str, path: &str) -> Item {
    Item::Folder {
        name: name.to_string(),
        path: path.to_string(),
        open: None,
        icon: None,
        show_branch: false,
    }
}

fn submenu(name: &str, items: Vec<Item>) -> Item {
    Item::Submenu {
        name: name.to_string(),
        items,
        show_branch: false,
    }
}

/// 深い階層に埋もれた項目でも、名前で見つかりメニューの完全パスが分かる
/// (プロジェクト数が多い環境で、どのサブメニューに入れたか覚えていなくても
/// 探せることが目的)。
#[test]
fn search_items_finds_a_nested_item_by_name_and_reports_its_menu_path() {
    let config = Config {
        items: vec![
            folder("alpha", "C:/alpha"),
            submenu(
                "Clients",
                vec![submenu(
                    "ClientA",
                    vec![folder("waypoint-project", "C:/work/waypoint-project")],
                )],
            ),
        ],
        ..Default::default()
    };

    let hits = search_items(&config, "waypoint");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].label, "waypoint-project");
    assert_eq!(hits[0].menu_name, "Clients > ClientA");
    assert_eq!(hits[0].menu_path, vec![1, 0]);
    assert_eq!(hits[0].index, 0);
}

/// パスの部分一致でも見つかる: 名前を覚えていなくてもフォルダパスの
/// 断片から探せる。
#[test]
fn search_items_also_matches_on_path() {
    let config = Config {
        items: vec![folder("Project", "C:/repos/needle-project")],
        ..Default::default()
    };
    let hits = search_items(&config, "needle");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].label, "Project");
}

#[test]
fn search_items_is_case_insensitive_and_trims_whitespace() {
    let config = Config {
        items: vec![folder("WayPoint", "C:/x")],
        ..Default::default()
    };
    assert_eq!(search_items(&config, "  waypoint  ").len(), 1);
}

#[test]
fn search_items_returns_nothing_for_an_empty_query() {
    let config = Config {
        items: vec![folder("alpha", "C:/alpha")],
        ..Default::default()
    };
    assert!(search_items(&config, "   ").is_empty());
}

/// 絞り込みは一致したメニューだけでなく、そこへ至る祖先も残す。
/// 祖先が消えると子だけがインデントされて浮き、階層が読めなくなる。
#[test]
fn filter_menu_choices_keeps_ancestors_of_a_match() {
    let config = Config {
        items: vec![submenu(
            "Clients",
            vec![
                submenu("ClientA", Vec::new()),
                submenu("ClientB", Vec::new()),
            ],
        )],
        ..Default::default()
    };
    let choices = menu_choices(&config);
    let names: Vec<&str> = filter_menu_choices(&choices, "ClientA")
        .into_iter()
        .map(|(_, name)| name.as_str())
        .collect();
    assert_eq!(names, vec!["Main", "Clients", "Clients > ClientA"]);
}

#[test]
fn filter_menu_choices_returns_everything_for_an_empty_query() {
    let config = Config {
        items: vec![submenu("A", Vec::new()), submenu("B", Vec::new())],
        ..Default::default()
    };
    let choices = menu_choices(&config);
    assert_eq!(filter_menu_choices(&choices, "").len(), choices.len());
}

/// 検索結果から飛ぶと、対象メニューへ切り替わり項目が選択され、
/// 検索は閉じて通常表示に戻る。
#[test]
fn jump_to_item_switches_menu_selects_the_item_and_closes_the_search() {
    let mut app = app_with_folders(0);
    app.config.items.push(submenu(
        "Sub",
        vec![folder("target", "C:/target"), folder("other", "C:/other")],
    ));
    app.search = "target".to_string();

    app.jump_to_item(vec![0], 0);

    assert_eq!(app.selected_menu, vec![0]);
    assert_eq!(app.selected_items, [0].into_iter().collect());
    assert_eq!(app.active_item, Some(0));
    assert!(app.search.is_empty());
}
