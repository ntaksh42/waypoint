//! メニューに使う Windows 標準アイコンが実際に取得できることを確認する。
//!
//! `SHGetStockIconInfo` は ID ごとに失敗しうる。失敗すると
//! アイコン無しで描かれるだけで気付きにくいため、選んだ ID が
//! この環境で有効かをテストで固定する。
//!
//! 1 つのテストにまとめてあるのは意図的。`ComGuard` の `Drop` が
//! `CoUninitialize` を呼ぶため、テストを分けると先に終わった側が
//! 同じスレッドの COM を落とし、後続が失敗する。アイコンの
//! キャッシュも thread_local で、失敗結果まで覚えてしまう。

use windows::Win32::UI::Shell::{
    SHSTOCKICONID, SIID_DESKTOPPC, SIID_DOCASSOC, SIID_DOCNOASSOC, SIID_FOLDER, SIID_FOLDEROPEN,
    SIID_STACK,
};

#[test]
fn menu_stock_icons_are_available_and_cached() {
    // 実アプリは main で COM を張っている。同じ前提を作る
    let _com = waypoint::shell::ComGuard::new();

    let ids: [(&str, SHSTOCKICONID); 6] = [
        ("SIID_FOLDER", SIID_FOLDER),
        ("SIID_FOLDEROPEN", SIID_FOLDEROPEN),
        ("SIID_STACK", SIID_STACK),
        ("SIID_DESKTOPPC", SIID_DESKTOPPC),
        ("SIID_DOCASSOC", SIID_DOCASSOC),
        ("SIID_DOCNOASSOC", SIID_DOCNOASSOC),
    ];

    let mut missing = Vec::new();
    for (name, id) in ids {
        if waypoint::icon::bitmap_for_stock(id).is_none() {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "取得できない標準アイコンがある: {missing:?}"
    );

    // 同じ ID を二度引いても同じハンドル。毎回作るとハンドルが増え続ける
    let first = waypoint::icon::bitmap_for_stock(SIID_FOLDER).expect("1 回目");
    let second = waypoint::icon::bitmap_for_stock(SIID_FOLDER).expect("2 回目");
    assert_eq!(first.0, second.0, "キャッシュが効いていない");

    // 用途ごとに別のアイコンであること (同じだとメニューで見分けが付かない)
    let distinct: Vec<isize> = [SIID_FOLDER, SIID_FOLDEROPEN, SIID_STACK, SIID_DESKTOPPC]
        .iter()
        .map(|id| {
            waypoint::icon::bitmap_for_stock(*id)
                .map(|bitmap| bitmap.0 as isize)
                .unwrap_or(0)
        })
        .collect();
    let mut unique = distinct.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        4,
        "同じビットマップが返っている: {distinct:?}"
    );
}
