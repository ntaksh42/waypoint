//! FR-2.14 / FR-2.15 のブランチ表示に関するテスト。

use waypoint::config::Item;

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
