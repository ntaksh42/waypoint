//! Azure DevOps の Area ツリー取得に対する実 API テスト。
//!
//! ネットワークと保存済み PAT を要るので既定では走らせない。
//! 手元で確認するときだけ明示的に呼ぶ:
//!
//! ```powershell
//! $env:WAYPOINT_ADO_ORG = "aksh0402"
//! $env:WAYPOINT_ADO_PROJECT = "TestProject"
//! cargo test --test azure_area_live_test -- --ignored --nocapture
//! ```
//!
//! 対象プロジェクトには次の階層がある前提 (az CLI で作成済み):
//!
//! ```text
//! TestProject
//! ├─ Launcher
//! │  ├─ Menu
//! │  │  ├─ Rendering
//! │  │  └─ Icons
//! │  ├─ Hotkey
//! │  └─ QuickLaunch
//! ├─ Settings
//! │  ├─ Trigger
//! │  └─ AzureDevOps
//! └─ Docs
//! ```

use waypoint::azure_devops::{AreaNode, fetch_area_nodes};

fn env_or_skip(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            eprintln!("skipped: {key} is not set");
            None
        }
    }
}

/// 設定画面と同じ絞り込み (フルパスへの部分一致)。UI 側の
/// `filter_area_nodes` と同じ規則をここでも使う。
fn filtered(nodes: &[AreaNode], filter: &str) -> Vec<String> {
    let needle = filter.to_lowercase();
    nodes
        .iter()
        .filter(|node| needle.is_empty() || node.path.to_lowercase().contains(&needle))
        .map(|node| node.path.clone())
        .collect()
}

#[test]
#[ignore = "requires network access and a saved Azure DevOps PAT"]
fn the_fetched_area_tree_is_flattened_with_project_relative_paths_and_depths() {
    let (Some(organization), Some(project)) = (
        env_or_skip("WAYPOINT_ADO_ORG"),
        env_or_skip("WAYPOINT_ADO_PROJECT"),
    ) else {
        return;
    };

    let nodes = fetch_area_nodes(&organization, &project, "").expect("area tree should load");
    for node in &nodes {
        println!("{}{}", "  ".repeat(node.depth), node.path);
    }

    // ルート (プロジェクト名だけ) は含めない。
    assert!(
        !nodes.iter().any(|node| node.path == project),
        "the root node must not be listed"
    );
    // API のレスポンスは `\TestProject\Area\Launcher` だが、アプリは
    // `name` から組み直すので `\Area` は現れない。
    assert!(
        !nodes.iter().any(|node| node.path.contains("\\Area\\")),
        "the classification-node \\Area segment must not leak into paths"
    );

    let launcher = nodes
        .iter()
        .find(|node| node.path == format!("{project}\\Launcher"))
        .expect("Launcher should exist");
    assert_eq!(launcher.depth, 0, "a top-level area sits at depth 0");

    let rendering = nodes
        .iter()
        .find(|node| node.path.ends_with("\\Menu\\Rendering"))
        .expect("Launcher\\Menu\\Rendering should exist");
    assert_eq!(rendering.depth, 2, "a third-level area sits at depth 2");
}

#[test]
#[ignore = "requires network access and a saved Azure DevOps PAT"]
fn filtering_by_a_parent_name_keeps_its_descendants() {
    let (Some(organization), Some(project)) = (
        env_or_skip("WAYPOINT_ADO_ORG"),
        env_or_skip("WAYPOINT_ADO_PROJECT"),
    ) else {
        return;
    };

    let nodes = fetch_area_nodes(&organization, &project, "").expect("area tree should load");

    // 親名で打つと、その配下がまとめて残る (フルパス一致のため)。
    let menu = filtered(&nodes, "menu");
    assert!(
        menu.iter().any(|path| path.ends_with("\\Menu")),
        "Menu itself should survive the filter: {menu:?}"
    );
    assert!(
        menu.iter().any(|path| path.ends_with("\\Menu\\Rendering")),
        "Menu's children should survive the filter: {menu:?}"
    );
    assert!(
        !menu.iter().any(|path| path.ends_with("\\Hotkey")),
        "an unrelated sibling must be filtered out: {menu:?}"
    );

    // 大文字小文字は区別しない。
    assert_eq!(filtered(&nodes, "MENU"), menu);

    // 空フィルタは素通し、一致なしは空。
    assert_eq!(filtered(&nodes, "").len(), nodes.len());
    assert!(filtered(&nodes, "no-such-area").is_empty());
}
