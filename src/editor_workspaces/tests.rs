use super::*;

#[test]
fn decodes_windows_uri_without_losing_special_characters() {
    assert_eq!(
        local_path("file:///e%3A/space%20and/%E6%97%A5%E6%9C%AC%E8%AA%9E/%23hash%25"),
        Some(r"E:\space and\日本語\#hash%".into())
    );
    assert_eq!(
        local_path("file://server/share/a%20b"),
        Some(r"\\server\share\a b".into())
    );
    assert_eq!(
        local_path("file://localhost/c:/project"),
        Some(r"C:\project".into())
    );
    for uri in [
        "https://example.com",
        "file:///relative",
        "file:///c:/bad%",
        "file:///c:/%FF",
        "file:///c:/%00",
    ] {
        assert_eq!(local_path(uri), None, "{uri}");
    }
}

#[test]
fn history_preserves_recency_and_excludes_regular_files() {
    let history = parse_history(r#"{"entries":[{"folderUri":"file:///c:/new"},{"fileUri":"file:///c:/file.rs"},{"workspace":{"id":"x","configPath":"file:///c:/old.code-workspace"}},{"folderUri":"vscode-remote://wsl+Ubuntu/home/me"}]}"#).unwrap();
    assert_eq!(
        history,
        vec![
            ("file:///c:/new".into(), false),
            ("file:///c:/old.code-workspace".into(), true),
            ("vscode-remote://wsl+Ubuntu/home/me".into(), false)
        ]
    );
    assert_eq!(parse_history("{}"), None);
    assert_eq!(parse_history(r#"{"entries":[]}"#), Some(vec![]));
}

#[test]
fn workspace_entries_keep_editor_and_uri() {
    let uri = "vscode-remote://wsl+Ubuntu/home/me/project.code-workspace";
    let workspace = parse_workspace(Editor::Cursor, uri, true).unwrap();
    assert!(workspace.remote);
    assert!(available(&workspace));
    let entries = entries(&[workspace]);
    assert_eq!(entries[0].name, "project");
    assert!(entries[0].breadcrumb.contains("Cursor · Remote"));
    assert_eq!(
        entries[0].action,
        crate::quick_launch::Action::OpenEditorWorkspace(Editor::Cursor, uri.into(), true)
    );
    assert_eq!(crate::quick_launch::prefix_badge("vs "), Some("WORKSPACES"));
    assert_eq!(
        crate::quick_launch::effective_search_term("vs project"),
        "project"
    );
}

#[test]
fn reads_sqlite_path_containing_hash_and_percent() {
    let root = std::env::temp_dir().join(format!("waypoint_workspaces_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let db_path = root.join("history#%.db");
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch("CREATE TABLE ItemTable (key TEXT, value TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO ItemTable VALUES (?1, ?2)",
        [
            "history.recentlyOpenedPathsList",
            r#"{"entries":[{"folderUri":"file:///c:/recent"}]}"#,
        ],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        read_history(&db_path),
        Some(vec![("file:///c:/recent".into(), false)])
    );
    std::fs::remove_file(db_path).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn filters_missing_local_targets_but_keeps_unc() {
    let missing = parse_workspace(
        Editor::Code,
        "file:///c:/waypoint_missing_workspace_29348234",
        false,
    )
    .unwrap();
    assert!(!available(&missing));
    assert!(available(
        &parse_workspace(Editor::Code, "file://unreachable/share/project", false).unwrap()
    ));
    let local = Workspace {
        path: env!("CARGO_MANIFEST_DIR").into(),
        ..missing
    };
    assert!(available(&local));
    assert!(!available(&Workspace {
        is_file: true,
        ..local
    }));
}

#[test]
fn absent_storage_is_empty_and_setting_defaults_on() {
    assert!(
        read_workspace_storage(Path::new(r"C:\waypoint_missing_workspace_29348234")).is_empty()
    );
    let settings: crate::config::QuickLaunchSettings = serde_json::from_str("{}").unwrap();
    assert!(settings.include_editor_workspaces);
    let settings: crate::config::QuickLaunchSettings =
        serde_json::from_str(r#"{"includeEditorWorkspaces":false}"#).unwrap();
    assert!(!settings.include_editor_workspaces);
}

#[test]
fn local_names_are_decoded_exactly_once_and_empty_search_keeps_recent_order() {
    let workspaces = ["file:///c:/z%2520%23", "file:///c:/a"]
        .map(|uri| parse_workspace(Editor::Code, uri, false).unwrap());
    let entries = entries(&workspaces);
    assert_eq!(entries[0].name, "z%20#");
    let snapshot = crate::quick_launch::Snapshot::new(entries);
    let ranking = crate::quick_launch_history::Ranking::default();
    let found = snapshot.search("", true, &ranking);
    assert_eq!(found[0].name, "z%20#");
    assert_eq!(found[1].name, "a");
    assert_eq!(snapshot.search("z%20", true, &ranking).len(), 1);
}

#[test]
#[ignore = "実機の読み取り件数・所要時間を確認する"]
fn scan_real_workspaces() {
    let start = Instant::now();
    let workspaces = scan();
    println!(
        "workspaces: {}, elapsed: {:?}",
        workspaces.len(),
        start.elapsed()
    );
    assert!(workspaces.iter().all(available));
}

#[test]
fn workspace_storage_uses_directory_recency_and_skips_broken_records() {
    use std::os::windows::fs::OpenOptionsExt;
    let root =
        std::env::temp_dir().join(format!("waypoint_workspace_storage_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for (name, json, seconds) in [
        ("older", r#"{"folder":"file:///c:/older"}"#, 100),
        (
            "newer",
            r#"{"workspace":"file:///c:/newer.code-workspace"}"#,
            200,
        ),
        ("broken", "{", 300),
    ] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("workspace.json"), json).unwrap();
        // ディレクトリ自身の日時を固定し、列挙順や待ち時間に依存させない。
        let handle = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(0x02000000)
            .open(&dir)
            .unwrap(); // FILE_FLAG_BACKUP_SEMANTICS
        handle
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            )
            .unwrap();
    }
    assert_eq!(
        read_workspace_storage(&root),
        vec![
            ("file:///c:/newer.code-workspace".into(), true),
            ("file:///c:/older".into(), false),
        ]
    );
    for name in ["older", "newer", "broken"] {
        let dir = root.join(name);
        std::fs::remove_file(dir.join("workspace.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}
