use super::super::azure::{AzureCommand, PipelineFilter, PullRequestFilter};
use super::super::*;
use super::fixture::{config_without_live_scans, index};
use crate::config::Config;
use crate::dynamic::Menus;

#[test]
fn all_terms_must_match_name_or_breadcrumb() {
    let index = index();
    let found = index.search("way rel");
    assert_eq!(
        found
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["Release"]
    );
}

#[test]
fn exact_and_prefix_matches_rank_before_substrings() {
    let index = index();
    let found = index.search("waypoint");
    // FR-9.2: 無接頭辞の通常検索は Folders / Open Windows / Bookmarks /
    // Apps を横断する (History は対象外、`h ` プレフィックス限定)。
    // 先頭一致 (Waypoint docs, waypoint - Notepad) が単語境界一致
    // (Old waypoint) や breadcrumb 一致 (Release) より先に並ぶ。
    assert_eq!(
        found
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Waypoint docs",
            "waypoint - Notepad",
            "Old waypoint",
            "Release"
        ]
    );
}

#[test]
fn previously_selected_entry_ranks_before_same_score_siblings() {
    // 両方とも prefix 一致で同スコアになる 2 件。並び順 (order) だけなら
    // Alpha が先に出るはずだが、Beta の選択履歴があれば逆転する
    let alpha = Entry {
        name: "Alpha Tools".into(),
        breadcrumb: String::new(),
        path: r"C:\Alpha".into(),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    };
    let beta = Entry {
        name: "Alpha Utils".into(),
        breadcrumb: String::new(),
        path: r"C:\Beta".into(),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    };
    let mut idx = Index {
        entries: vec![alpha, beta.clone()],
        ..Index::default()
    };
    idx.ranking = Ranking::default().with_selection(&beta, 3, 100);

    let found = idx.search("alpha");
    assert_eq!(
        found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["Alpha Utils", "Alpha Tools"]
    );
}

#[test]
fn path_search_is_opt_in() {
    let mut index = index();
    assert!(index.search("target").is_empty());
    index.search_paths = true;
    assert_eq!(index.search("target")[0].name, "Release");
}

#[test]
fn window_prefix_switches_to_window_only_search() {
    let index = index();
    let found = index.search("w notepad");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, Action::FocusWindow(12345));
}

/// FR-9.2: `w ` プレフィックスなしでも Open Windows は通常検索の対象。
#[test]
fn open_windows_are_searched_without_the_window_prefix() {
    let index = index();
    let found = index.search("notepad");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, Action::FocusWindow(12345));
}

/// タイトルにアプリ名が出ないウィンドウも、所有プロセス名で
/// 見つけられる (`w chrome` のような検索)。
#[test]
fn window_search_also_matches_the_owning_process_name() {
    use crate::dynamic::WindowEntry;

    let dynamic = Menus {
        all_windows: vec![WindowEntry {
            title: "新しいタブ".to_string(),
            hwnd: 999,
            process_name: "chrome.exe".to_string(),
        }],
        ..Menus::default()
    };
    let index = Index::build(&Config::default(), &dynamic);
    let found = index.search("w chrome");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, Action::FocusWindow(999));
}

/// トレイの "Current Windows" 表示は件数を絞るが (ITEM_LIMIT) 、
/// Quick Launch の検索索引は絞られていない全件から作られること。
/// 絞られた current_windows だけを索引に使うと、開いているウィンドウが
/// 多い環境で一部が `w ` 検索に一切ヒットしなくなる (実際の不具合)。
#[test]
fn window_search_uses_all_windows_not_the_truncated_tray_list() {
    use crate::dynamic::WindowEntry;

    let window = |hwnd: isize| WindowEntry {
        title: format!("Window {hwnd}"),
        hwnd,
        process_name: "app.exe".to_string(),
    };

    let dynamic = Menus {
        // トレイ表示用は 1 件だけに絞られているとする
        current_windows: vec![window(1)],
        // 検索索引用は絞られていない全件
        all_windows: vec![window(1), window(2), window(3)],
        ..Menus::default()
    };
    let index = Index::build(&Config::default(), &dynamic);
    let found = index.search("w window");
    assert_eq!(found.len(), 3);
}

#[test]
fn apps_prefix_switches_to_apps_only_search() {
    let index = index();
    let found = index.search("a code");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, Action::LaunchApp);
}

#[test]
fn tabs_prefix_searches_the_live_tab_cache_only() {
    let mut index = index();
    index.set_browser_tabs(&[(
        crate::browser_tabs::Browser::Chrome,
        crate::browser_tabs::Tab {
            id: 7,
            window_id: 3,
            title: "Waypoint documentation".into(),
            url: "https://example.test/waypoint".into(),
        },
    )]);

    let found = index.search("t example.test");
    assert_eq!(found.len(), 1);
    assert!(matches!(
        found[0].action,
        Action::FocusBrowserTab(crate::browser_tabs::TabTarget {
            browser: crate::browser_tabs::Browser::Chrome,
            tab_id: 7,
            window_id: 3,
        })
    ));
    assert!(index.search("example.test").is_empty());
}

/// FR-9.2: `a ` プレフィックスなしでもアプリは通常検索の対象。
#[test]
fn apps_are_searched_without_the_apps_prefix() {
    let index = index();
    let found = index.search("code");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].action, Action::LaunchApp);
}

/// 絞り込みなしの一覧は、データを持つ区分だけを由来別に分けて返す。
/// フィクスチャの `index()` は Azure DevOps も持つが、区分見出し
/// 一覧には含めない (プレフィックス検索でしか出さない設計)。
#[test]
fn sections_group_results_by_source_and_skip_empty_ones() {
    let index = index();
    let sections = index.sections();
    assert_eq!(
        sections.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
        ["Folders", "Open Windows", "Bookmarks", "History", "Apps"]
    );
    let folders = &sections
        .iter()
        .find(|(label, _)| *label == "Folders")
        .unwrap()
        .1;
    assert_eq!(folders.len(), 3);
}

/// 区分ごとの件数を絞る (一覧が縦に伸びすぎないようにするため)。
#[test]
fn sections_cap_each_source_at_the_section_limit() {
    let mut index = index();
    index.entries = (0..10)
        .map(|n| Entry {
            name: format!("Folder {n}"),
            breadcrumb: String::new(),
            path: format!(r"C:\folder{n}"),
            action: Action::OpenFolder(OpenMode::NewWindow),
            branch: None,
        })
        .collect();

    let sections = index.sections();
    let folders = &sections
        .iter()
        .find(|(label, _)| *label == "Folders")
        .unwrap()
        .1;
    assert_eq!(folders.len(), 6);
}

#[test]
fn bookmark_prefix_switches_to_bookmark_only_search() {
    let index = index();
    let found = index.search("b git");
    assert_eq!(
        found
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["GitHub"]
    );
    assert_eq!(
        found[0].action,
        Action::OpenUrl("https://github.com/".into())
    );
}

/// FR-9.2: `b ` プレフィックスなしでもブックマークは通常検索の対象。
#[test]
fn bookmarks_are_searched_without_the_bookmark_prefix() {
    let index = index();
    let found = index.search("github");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "GitHub");
}

#[test]
fn bookmark_search_matches_the_url_too() {
    let index = index();
    let found = index.search("b example.com");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Example");
}

#[test]
fn history_prefix_switches_to_history_only_search() {
    let index = index();
    let found = index.search("h github.com/example");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "WayPoint pull request");
}

/// ブラウザ履歴は件数が多く、通常検索に含めるとキー入力ごとの全件走査が
/// 重くなるため対象外 (`h ` プレフィックス限定)。
#[test]
fn without_the_history_prefix_history_is_not_searched() {
    let index = index();
    assert!(index.search("pull request").is_empty());
}

#[test]
fn azure_pr_status_command_filters_cached_pull_requests() {
    let index = index();
    assert_eq!(index.search("az pr active azure").len(), 1);
    assert_eq!(index.search("az pr active mine azure").len(), 1);
    assert!(index.search("az pr completed azure").is_empty());
    assert_eq!(index.search("az wp").len(), 1);
}

#[test]
fn cached_work_items_are_searchable_without_live_api() {
    let mut index = index();
    index.azure_work_items = vec![Entry {
        name: "91: Cache WIT results".into(),
        breadcrumb: "Azure DevOps — org/Waypoint — Bug Active".into(),
        path: "https://dev.azure.com/org/Waypoint/_workitems/edit/91".into(),
        action: Action::OpenUrl("https://dev.azure.com/org/Waypoint/_workitems/edit/91".into()),
        branch: None,
    }];
    index.azure_work_items_lower =
        crate::quick_launch::search::LowerKeys::build_for(&index.azure_work_items);

    let found = index.search_cached_work_items("cache");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "91: Cache WIT results");
    assert!(index.search_cached_work_items("missing").is_empty());

    let all_azure = index.search("az cache");
    assert_eq!(all_azure.len(), 1);
    assert_eq!(all_azure[0].name, "91: Cache WIT results");
}

#[test]
fn shell_items_are_indexed_and_open_with_default_handler() {
    let config = Config {
        items: vec![Item::Shell {
            name: "This PC".to_string(),
            target: "shell:MyComputerFolder".to_string(),
        }],
        ..config_without_live_scans()
    };
    let index = Index::build(&config, &Menus::default());
    let found = index.search("this pc");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "shell:MyComputerFolder");
    assert_eq!(found[0].action, Action::OpenWithDefaultHandler);
}

#[test]
fn file_items_are_indexed_and_open_with_default_handler() {
    let config = Config {
        items: vec![Item::File {
            name: "Notes".to_string(),
            path: r"E:\notes.txt".to_string(),
            icon: None,
        }],
        ..config_without_live_scans()
    };
    let index = Index::build(&config, &Menus::default());
    let found = index.search("notes");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, r"E:\notes.txt");
    assert_eq!(found[0].action, Action::OpenWithDefaultHandler);
}

#[test]
fn same_path_from_config_and_recent_and_frequent_folds_into_one() {
    use crate::dynamic::PathEntry;

    let config = Config {
        items: vec![Item::Folder {
            name: "DevDeck".to_string(),
            path: r"E:\DevDeck".to_string(),
            open: None,
            icon: None,
            show_branch: false,
        }],
        ..config_without_live_scans()
    };
    let dynamic = Menus {
        recent_folders: vec![PathEntry {
            name: "DevDeck".to_string(),
            path: r"E:\DevDeck".to_string(),
        }],
        frequent_folders: vec![PathEntry {
            name: "DevDeck".to_string(),
            path: r"e:\devdeck".to_string(), // 大文字小文字違いでも同一視する
        }],
        ..Menus::default()
    };
    let index = Index::build(&config, &dynamic);
    let found = index.search("devdeck");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].breadcrumb, ""); // config.items 直下 = breadcrumb なし
}

#[test]
fn prefix_badge_identifies_each_mode() {
    assert_eq!(prefix_badge("b git"), Some("BOOKMARKS"));
    assert_eq!(prefix_badge("h waypoint"), Some("HISTORY"));
    assert_eq!(prefix_badge("az pr active waypoint"), Some("AZURE DEVOPS"));
    assert_eq!(prefix_badge("w notepad"), Some("WINDOWS"));
    assert_eq!(prefix_badge("a code"), Some("APPS"));
    assert_eq!(prefix_badge("t waypoint"), Some("TABS"));
    assert_eq!(prefix_badge("f cargo.toml"), Some("FILES"));
    assert_eq!(prefix_badge("plain query"), None);
    assert_eq!(prefix_badge(""), None);
}

#[test]
fn azure_command_recognizes_all_supported_subcommands() {
    assert_eq!(
        azure_command("az pr completed done"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Completed,
                mine: false,
            }),
            "done"
        ))
    );
    assert_eq!(
        azure_command("az wit bug"),
        Some((AzureCommand::WorkItems, "bug"))
    );
    assert_eq!(
        azure_command("az pipelines release"),
        Some((AzureCommand::Pipelines(PipelineFilter::All), "release"))
    );
    assert_eq!(
        azure_command("az pipeline failed release"),
        Some((AzureCommand::Pipelines(PipelineFilter::Failed), "release"))
    );
    assert_eq!(
        azure_command("az pr active mine launcher"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Active,
                mine: true,
            }),
            "launcher"
        ))
    );
    // 属性トークンは順不同で並べられる。
    assert_eq!(
        azure_command("az pr mine active launcher"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Active,
                mine: true,
            }),
            "launcher"
        ))
    );
    // 属性トークンだけで検索語が無ければ空文字列になる。
    assert_eq!(
        azure_command("az pr active"),
        Some((
            AzureCommand::PullRequests(PullRequestFilter {
                status: crate::azure_devops::PullRequestStatus::Active,
                mine: false,
            }),
            ""
        ))
    );
    assert_eq!(
        azure_command("az workitems defect"),
        Some((AzureCommand::WorkItems, "defect"))
    );
    assert_eq!(
        azure_command("az platform"),
        Some((AzureCommand::All, "platform"))
    );
}

#[test]
fn azure_prefix_alone_shows_nothing() {
    let index = index();
    assert!(index.search("az ").is_empty());
}

#[test]
fn incomplete_azure_command_uses_fuzzy_completion() {
    let index = index();
    let found = index.search("az pln");
    assert_eq!(
        found
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["az pipeline"]
    );
    // コマンド候補に当たらない文字列は、従来どおり Azure 全体を検索する。
    assert_eq!(index.search("az wp")[0].name, "PR 42: Add Azure search");
}
