//! FR-2.14 / FR-2.15 のブランチ表示に関するテスト。

use std::path::{Path, PathBuf};

use waypoint::config::Item;
use waypoint::git::{parse_head, resolve_gitdir_file, with_branch};

/// このリポジトリ自身を対象にする。作業ツリー内なら何らかの名前が取れる。
#[test]
fn reads_branch_of_this_repository() {
    let here = env!("CARGO_MANIFEST_DIR");
    let branch = waypoint::git::branch_of(here);
    assert!(
        branch.is_some(),
        "リポジトリ直下でブランチ名を取得できるはず: {here}"
    );
    let branch = branch.unwrap();
    assert!(!branch.is_empty());
    assert!(!branch.contains('\n'), "改行を含めない: {branch:?}");
}

/// サブディレクトリを登録していても祖先の `.git` を見つける。
#[test]
fn finds_repository_from_subdirectory() {
    let sub = format!("{}\\src", env!("CARGO_MANIFEST_DIR"));
    assert_eq!(waypoint::git::branch_of(&sub), {
        waypoint::git::branch_of(env!("CARGO_MANIFEST_DIR"))
    });
}

/// リポジトリでない場所では何も返さない (項目名に何も付かない)。
#[test]
fn returns_none_outside_repository() {
    let temp = std::env::temp_dir();
    // temp 配下がリポジトリのことは通常ないが、念のため祖先も確認する
    if temp.ancestors().any(|d| d.join(".git").exists()) {
        return;
    }
    assert_eq!(waypoint::git::branch_of(&temp.display().to_string()), None);
}

/// 存在しないパスでもパニックせず None を返す。
#[test]
fn returns_none_for_missing_path() {
    assert_eq!(
        waypoint::git::branch_of("Z:\\no\\such\\folder\\at\\all"),
        None
    );
}

/// 既定は偽で、偽のときは JSON に出力しない (既存の config を汚さない)。
#[test]
fn show_branch_defaults_to_false_and_is_omitted() {
    let json = r#"{"type":"folder","name":"P","path":"C:\\p"}"#;
    let item: Item = serde_json::from_str(json).expect("既存形式を読めること");
    let Item::Folder { show_branch, .. } = &item else {
        panic!("folder として読めること");
    };
    assert!(!show_branch, "既定は偽");

    let out = serde_json::to_string(&item).unwrap();
    assert!(!out.contains("showBranch"), "偽なら出力しない: {out}");
}

/// 真のときは camelCase で往復する。
#[test]
fn show_branch_round_trips_when_true() {
    let json = r#"{"type":"folder","name":"P","path":"C:\\p","showBranch":true}"#;
    let item: Item = serde_json::from_str(json).unwrap();
    let Item::Folder { show_branch, .. } = &item else {
        panic!("folder として読めること");
    };
    assert!(show_branch);

    let out = serde_json::to_string(&item).unwrap();
    assert!(out.contains(r#""showBranch":true"#), "{out}");
}

#[test]
fn appends_branch_when_present() {
    assert_eq!(with_branch("waypoint", Some("main")), "waypoint  [main]");
}

#[test]
fn leaves_name_alone_outside_repository() {
    assert_eq!(with_branch("Downloads", None), "Downloads");
}

#[test]
fn parses_branch_ref() {
    assert_eq!(
        parse_head("ref: refs/heads/main\n").as_deref(),
        Some("main")
    );
}

#[test]
fn keeps_slashes_in_branch_name() {
    assert_eq!(
        parse_head("ref: refs/heads/feature/git-branch\n").as_deref(),
        Some("feature/git-branch")
    );
}

#[test]
fn shortens_detached_head_sha() {
    assert_eq!(
        parse_head("9f8e7d6c5b4a39281706zz").as_deref(),
        None,
        "SHA 以外の文字が混ざるものは表示しない"
    );
    assert_eq!(
        parse_head("9f8e7d6c5b4a3928170695e4d3c2b1a098765432\n").as_deref(),
        Some("9f8e7d6")
    );
}

#[test]
fn ignores_non_branch_refs() {
    assert_eq!(parse_head("ref: refs/tags/v1.0\n"), None);
    assert_eq!(parse_head(""), None);
    assert_eq!(parse_head("ref: refs/heads/\n"), None);
}

#[test]
fn resolves_relative_gitdir_file() {
    let resolved = resolve_gitdir_file(
        "gitdir: ../.git/worktrees/feature\n",
        Path::new("C:\\work\\repo-feature"),
    );
    assert_eq!(
        resolved,
        Some(PathBuf::from(
            "C:\\work\\repo-feature/../.git/worktrees/feature"
        ))
    );
}

#[test]
fn resolves_absolute_gitdir_file() {
    let resolved = resolve_gitdir_file(
        "gitdir: C:\\work\\repo\\.git\\worktrees\\feature\n",
        Path::new("C:\\ignored"),
    );
    assert_eq!(
        resolved,
        Some(PathBuf::from("C:\\work\\repo\\.git\\worktrees\\feature"))
    );
}

#[test]
fn rejects_gitdir_file_without_target() {
    assert_eq!(resolve_gitdir_file("gitdir:\n", Path::new("C:\\x")), None);
    assert_eq!(
        resolve_gitdir_file("something else\n", Path::new("C:\\x")),
        None
    );
}

/// Submenu の showBranch も Folder と同じ規則 (既定は偽・省略) で往復する。
#[test]
fn submenu_show_branch_defaults_to_false_and_is_omitted() {
    let json = r#"{"type":"submenu","name":"E:\\","items":[]}"#;
    let item: Item = serde_json::from_str(json).expect("既存形式を読めること");
    let Item::Submenu { show_branch, .. } = &item else {
        panic!("submenu として読めること");
    };
    assert!(!show_branch, "既定は偽");

    let out = serde_json::to_string(&item).unwrap();
    assert!(!out.contains("showBranch"), "偽なら出力しない: {out}");
}

/// 真のときは camelCase で往復する。配下の Folder への継承は
/// menu.rs / quick_launch.rs 側のロジックで検証する。
#[test]
fn submenu_show_branch_round_trips_when_true() {
    let json = r#"{"type":"submenu","name":"E:\\","items":[],"showBranch":true}"#;
    let item: Item = serde_json::from_str(json).unwrap();
    let Item::Submenu { show_branch, .. } = &item else {
        panic!("submenu として読めること");
    };
    assert!(show_branch);

    let out = serde_json::to_string(&item).unwrap();
    assert!(out.contains(r#""showBranch":true"#), "{out}");
}
