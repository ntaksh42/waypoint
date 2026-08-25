//! メニューに使うアイコンが実際に「絵として」取得できることを確認する。
//!
//! ハンドルが返るだけでは不十分。`SHGetStockIconInfo` は ID によって
//! 全ピクセルが透明なアイコンを返すことがあり (実測: `SIID_SETTINGS`)、
//! その場合メニューには何も表示されないまま気付けない。
//! 不透明ピクセルが 1 つ以上あることまで見る。
//!
//! 1 つのテストにまとめてあるのは意図的。`ComGuard` の `Drop` が
//! `CoUninitialize` を呼ぶため、テストを分けると先に終わった側が
//! 同じスレッドの COM を落とし、後続が失敗する。アイコンの
//! キャッシュも thread_local で、失敗結果まで覚えてしまう。

use windows::Win32::Graphics::Gdi::{BITMAP, GetObjectW, HBITMAP};
use windows::Win32::UI::Shell::{
    SHSTOCKICONID, SIID_DESKTOPPC, SIID_DOCASSOC, SIID_DOCNOASSOC, SIID_FOLDER, SIID_FOLDEROPEN,
    SIID_STACK,
};

/// ビットマップの不透明ピクセル数。0 なら見た目は空。
fn opaque_pixels(bitmap: HBITMAP) -> usize {
    unsafe {
        let mut info = BITMAP::default();
        let read = GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some(std::ptr::from_mut(&mut info).cast()),
        );
        if read == 0 || info.bmBits.is_null() {
            return 0;
        }
        let len = (info.bmWidth * info.bmHeight * 4) as usize;
        let bits = std::slice::from_raw_parts(info.bmBits.cast::<u8>(), len);
        bits.as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] != 0)
            .count()
    }
}

#[test]
fn menu_icons_are_available_and_visible() {
    // 実アプリは main で COM を張っている。同じ前提を作る
    let _com = waypoint::shell::ComGuard::new();

    let stock: [(&str, SHSTOCKICONID); 6] = [
        ("SIID_FOLDER", SIID_FOLDER),
        ("SIID_FOLDEROPEN", SIID_FOLDEROPEN),
        ("SIID_STACK", SIID_STACK),
        ("SIID_DESKTOPPC", SIID_DESKTOPPC),
        ("SIID_DOCASSOC", SIID_DOCASSOC),
        ("SIID_DOCNOASSOC", SIID_DOCNOASSOC),
    ];

    let mut blank = Vec::new();
    for (name, id) in stock {
        match waypoint::icon::bitmap_for_stock(id) {
            Some(bitmap) if opaque_pixels(bitmap) > 0 => {}
            Some(_) => blank.push(format!("{name} (空)")),
            None => blank.push(format!("{name} (取得失敗)")),
        }
    }

    // 歯車は標準 ID から取れないので DLL から引いている
    match waypoint::icon::bitmap_for_settings() {
        Some(bitmap) if opaque_pixels(bitmap) > 0 => {}
        Some(_) => blank.push("settings gear (空)".to_string()),
        None => blank.push("settings gear (取得失敗)".to_string()),
    }

    assert!(
        blank.is_empty(),
        "絵として使えないアイコンがある: {blank:?}"
    );

    // 同じ ID を二度引いても同じハンドル。毎回作るとハンドルが増え続ける
    let first = waypoint::icon::bitmap_for_stock(SIID_FOLDER).expect("1 回目");
    let second = waypoint::icon::bitmap_for_stock(SIID_FOLDER).expect("2 回目");
    assert_eq!(first.0, second.0, "キャッシュが効いていない");
    let gear_first = waypoint::icon::bitmap_for_settings().expect("歯車 1 回目");
    let gear_second = waypoint::icon::bitmap_for_settings().expect("歯車 2 回目");
    assert_eq!(
        gear_first.0, gear_second.0,
        "歯車のキャッシュが効いていない"
    );

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
