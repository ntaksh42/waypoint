//! 索引構築・ファイル / COM I/O・描画のベンチ。
//!
//! 検索そのものではなく「表示や選択の前後で何が走るか」を測る。
//! 実運用のボトルネックは検索ではなくこちら側にあった
//! (`Index::build` 91ms / 35ms 対 軽量版 0.001ms、
//! 使用履歴の同期保存 18.8ms など)。

use super::super::*;
use super::bench_fixture::large_index;

/// 索引構築のうち、動的な差し替え (`refresh_dynamic`) で毎回走る部分。
/// Recent/Frequent Folders と開いているウィンドウが変わるたびに走る。
#[test]
#[ignore = "手動計測用"]
fn bench_index_rebuild() {
    use std::time::Instant;
    let index = large_index(2000, 3000, 5000, 500);

    let entries = index.entries.clone();
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(super::super::search::LowerKeys::build_for(&entries));
    }
    println!(
        "LowerKeys::build_for({})  {:>7.3} ms",
        entries.len(),
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );

    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(super::super::search::bench_dedup(entries.clone()));
    }
    let dedup = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(entries.clone());
    }
    let clone_only = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
    println!(
        "dedup_by_path({})         {:>7.3} ms  (clone {:.3} ms)",
        entries.len(),
        dedup - clone_only,
        clone_only
    );

    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(Ranking::default());
    }
    println!(
        "Ranking::default()        {:>7.3} ms",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0
    );
}

/// スタートメニューの `.lnk` 列挙 (`a ` プレフィックスの母集団)。
/// `Index::build` で走る。実機のスタートメニューをそのまま読む。
///
/// コストのほぼ全部 (実測 30ms 中 27ms) が `.lnk` の実体解決で、
/// 内訳は `IPersistFile::Load` のファイル I/O。COM オブジェクトを
/// 1 個に使い回す・受け取りバッファを再利用するといった手は試したが
/// 速くならなかった (実測で差が出ず、複雑さだけ増えたので入れていない)。
/// `SLGP_RAWPATH` は環境変数が展開されず、正しいアプリまで壊れた
/// リンク扱いで落ちるので使えない (実測: 87 件 → 45 件)。
///
/// したがってここは「速くする」より「UI スレッドで何度も走らせない」
/// 方向で抑える (`Index::refresh_azure` / `refresh_dynamic` 参照)。
#[test]
#[ignore = "手動計測用"]
fn bench_apps_scan() {
    use std::time::Instant;
    // COM を使うので初期化しておく (常駐部は UI スレッドで初期化済み)。
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let start = Instant::now();
    let apps = crate::apps::scan();
    let first = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let again = crate::apps::scan();
    let second = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "apps::scan() {} 件  1 回目 {first:>8.2} ms  2 回目 {second:>8.2} ms",
        apps.len()
    );
    assert_eq!(apps.len(), again.len());
}

/// フル `Index::build` の実測。実機のブックマーク・履歴・スタートメニューを
/// 読むので、起動時と設定リロード時のコストがそのまま出る。
///
/// この経路をユーザー操作のたびに通さないことが重要で、そのために
/// `refresh_dynamic` / `refresh_azure` / `refresh_config_items` がある。
#[test]
#[ignore = "手動計測用"]
fn bench_index_build_full() {
    use crate::config::Config;
    use crate::dynamic::Menus;
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let config = Config::default();
    let dynamic = Menus::default();

    let start = Instant::now();
    let index = Index::build(&config, &dynamic);
    let cold = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let warm_index = Index::build(&config, &dynamic);
    let warm = start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "Index::build 1 回目 {cold:>8.2} ms  2 回目 {warm:>8.2} ms  (entries={} bookmarks={} history={} apps={})",
        index.entries.len(),
        index.bookmarks.len(),
        index.history.len(),
        index.apps.len(),
    );

    // 軽量版との比較
    let mut index = warm_index;
    let start = Instant::now();
    index.refresh_dynamic(&config, &dynamic);
    let dynamic_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    index.refresh_config_items(&config, &dynamic);
    let config_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("refresh_dynamic {dynamic_ms:>8.3} ms  refresh_config_items {config_ms:>8.3} ms");
}

/// 1 行描くたびに走る `Entry` の複製。`draw_list_item` は借用を解放してから
/// 描くために結果を clone しており、再描画のたびに行数ぶん走る
/// (`LBS_OWNERDRAWVARIABLE` なので打鍵ごとの `LB_RESETCONTENT` で全行再描画)。
///
/// 結論: 再描画 1 回ぶんで 0.0016ms しかかからないので手を入れない。
/// この clone は再入 (`RefCell` の借用を解放してから描く) のために
/// あるので、消すと `quick_launch_reentrancy_test` が守っている性質を
/// 壊す。コストが無視できる以上、触る理由がない。
#[test]
#[ignore = "手動計測用"]
fn bench_row_entry_clone() {
    use std::time::Instant;
    const VISIBLE_ROWS: usize = 24;
    let index = large_index(2000, 3000, 5000, 500);
    let results: Vec<Entry> = index
        .search("project")
        .into_iter()
        .take(VISIBLE_ROWS)
        .cloned()
        .collect();

    let start = Instant::now();
    for _ in 0..2000 {
        // 1 回の再描画ぶん (可視行すべて)
        for entry in &results {
            std::hint::black_box(entry.clone());
        }
    }
    println!(
        "Entry::clone x{VISIBLE_ROWS} (再描画 1 回ぶん) {:>8.4} ms",
        start.elapsed().as_secs_f64() * 1000.0 / 2000.0
    );
}

/// アイコンのキャッシュ参照キー。`bitmap_for_sized` 等が呼び出しごとに
/// `format!` でキーを作るため、再描画のたびに行数ぶんの確保とハッシュが走る。
///
/// 結論: 再描画 1 回ぶん (24 行) で 0.003ms。確保を使い回しても 0.001ms に
/// なるだけで、キーの組み立てを呼び出し側へ引き回す複雑さに見合わない。
/// 手を入れない判断の根拠として計測値だけ残す。
#[test]
#[ignore = "手動計測用"]
fn bench_icon_cache_key() {
    use std::collections::HashMap;
    use std::time::Instant;
    const VISIBLE_ROWS: usize = 24;
    let paths: Vec<String> = (0..VISIBLE_ROWS)
        .map(|i| format!(r"E:\projects\group{}\project-{i}\src", i % 8))
        .collect();
    let mut cache: HashMap<String, isize> = HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        cache.insert(format!("{}:{path}", 32), i as isize);
    }

    // 現行: format! でキーを作って引く
    let start = Instant::now();
    for _ in 0..2000 {
        let mut found = 0;
        for path in &paths {
            let key = format!("{}:{path}", 32);
            if cache.contains_key(&key) {
                found += 1;
            }
        }
        std::hint::black_box(found);
    }
    let with_format = start.elapsed().as_secs_f64() * 1000.0 / 2000.0;

    // 比較: 確保を使い回した場合 (1 本の String をクリアして詰め直す)
    let start = Instant::now();
    let mut key = String::new();
    for _ in 0..2000 {
        let mut found = 0;
        for path in &paths {
            key.clear();
            key.push_str("32:");
            key.push_str(path);
            if cache.contains_key(key.as_str()) {
                found += 1;
            }
        }
        std::hint::black_box(found);
    }
    let reused = start.elapsed().as_secs_f64() * 1000.0 / 2000.0;

    println!(
        "icon cache key x{VISIBLE_ROWS}: format! {with_format:>8.5} ms  /  確保使い回し {reused:>8.5} ms"
    );
}

/// Recent/Frequent Folders と開いているウィンドウの列挙 (`dynamic::refresh`)。
/// メニューを閉じるたびにバックグラウンドスレッドで走る。UI は塞がないが、
/// 遅すぎると次回表示までに結果が間に合わない。
///
/// 内訳 (実測、Recent 33 件): `scan_recent_items` が 27.6ms でほぼ全部。
/// `apps::scan` と同じく `.lnk` の COM 実体解決が理由。履歴の読み書きは
/// 0.05ms、`enumerate_windows` は 0.15ms、`frequent_entries` は 0.35ms で
/// いずれも誤差。
///
/// `refresh_async` が再入ガード付きでバックグラウンドに逃がし、UI 側は
/// 軽量な `configure_dynamic` しか通らないので、この 44ms は表示経路には
/// 乗らない。手を入れる対象ではない。
#[test]
#[ignore = "手動計測用"]
fn bench_dynamic_refresh() {
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let menus = crate::dynamic::refresh(); // ウォームアップ
    let start = Instant::now();
    let menus2 = crate::dynamic::refresh();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "dynamic::refresh {ms:>8.2} ms  (recent={} frequent={} windows={})",
        menus2.recent_folders.len(),
        menus2.frequent_folders.len(),
        menus2.all_windows.len(),
    );
    assert_eq!(menus.all_windows.len(), menus2.all_windows.len());
}

/// 候補を選んだ直後に走る使用履歴の記録 (`quick_launch_history::record`)。
///
/// `WM_QUICK_LAUNCH_EXECUTE` の中で、実際にフォルダやアプリを開く**前**に
/// 同期で呼ばれる。JSON 全体の読み込み → 更新 → 原子的保存 (temp→replace) を
/// 行うため、ここが遅いとユーザーの体感 (選んでから開くまで) に直接乗る。
#[test]
#[ignore = "手動計測用"]
fn bench_history_record() {
    use std::time::Instant;
    let entry = Entry {
        name: "Waypoint".into(),
        breadcrumb: "Projects".into(),
        path: r"E:\waypoint".into(),
        action: Action::OpenFolder(crate::config::OpenMode::NewWindow),
        branch: None,
    };
    // 実機の履歴ファイルをそのまま使う (件数がコストに効く)
    for _ in 0..3 {
        crate::quick_launch_history::record_blocking(&entry);
    }
    let start = Instant::now();
    for _ in 0..20 {
        crate::quick_launch_history::record_blocking(&entry);
    }
    println!(
        "quick_launch_history::record {:>8.3} ms/回 (選択のたびに UI スレッドで同期実行)",
        start.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
}

/// `record` の内訳。load / update / save のどこに 15ms が乗っているか。
#[test]
#[ignore = "手動計測用"]
fn bench_history_record_breakdown() {
    use crate::quick_launch_history::bench_parts;
    use std::time::Instant;

    let entry = Entry {
        name: "Waypoint".into(),
        breadcrumb: "Projects".into(),
        path: r"E:\waypoint".into(),
        action: Action::OpenFolder(crate::config::OpenMode::NewWindow),
        branch: None,
    };
    macro_rules! part {
        ($label:expr, $body:expr) => {{
            for _ in 0..3 {
                std::hint::black_box($body);
            }
            let start = Instant::now();
            for _ in 0..20 {
                std::hint::black_box($body);
            }
            println!(
                "{:<26} {:>8.3} ms",
                $label,
                start.elapsed().as_secs_f64() * 1000.0 / 20.0
            );
        }};
    }
    part!("load (JSON 読み込み)", bench_parts::load_len());
    part!("save (原子的保存)", bench_parts::save_roundtrip());
    part!("record 全体", {
        crate::quick_launch_history::record_blocking(&entry);
        0
    });
}

/// `record_async` がバックグラウンドスレッドでも確実に書き込むかの確認。
/// 実機の履歴ファイルを更新するので、ベンチ扱い (`#[ignore]`) にしてある。
#[test]
#[ignore = "実機の履歴を更新するため手動実行"]
fn bench_record_async_persists() {
    use std::time::Instant;
    let entry = Entry {
        name: "Waypoint".into(),
        breadcrumb: "Projects".into(),
        path: r"E:\waypoint".into(),
        action: Action::OpenFolder(crate::config::OpenMode::NewWindow),
        branch: None,
    };
    // 呼び出し側が待たされないことを見る (スレッド生成のみ)
    let start = Instant::now();
    crate::quick_launch_history::record_async(&entry);
    let call = start.elapsed().as_secs_f64() * 1000.0;
    println!("record_async の呼び出し側 {call:>8.4} ms (書き込みは待たない)");

    // 書き込み完了を待ってから結果を確認する
    std::thread::sleep(std::time::Duration::from_millis(500));
    let ranking = crate::quick_launch_history::Ranking::load();
    let (count_rank, _) = ranking.rank_lower(&entry, &entry.path.to_lowercase());
    assert_ne!(
        count_rank,
        u64::MAX,
        "record_async の書き込みが履歴に反映されていない"
    );
    println!("書き込み後の count = {}", u64::MAX - count_rank);
}

/// お気に入り登録 (`Ctrl+Shift+Enter`) で走る設定ファイルの保存。
/// `config::save` も `write_atomic` 経由なのでディスク flush を伴う。
///
/// 実測 6.08ms。`quick_launch_history::record` (18.8ms) と同じ形だが、
/// **こちらは非同期にしない。** 履歴は次回以降の並び順にしか効かない
/// カウンタなので落としても実害が無いのに対し、config はユーザーが今
/// 追加した項目そのもの。書き終わる前にプロセスが落ちれば「登録したのに
/// 消えた」になる。頻度も「選択のたび」ではなく明示操作のときだけなので、
/// 6ms を同期で払う方が妥当。
#[test]
#[ignore = "手動計測用"]
fn bench_config_save() {
    use std::time::Instant;
    let config = match crate::config::load() {
        crate::config::LoadOutcome::Loaded(config)
        | crate::config::LoadOutcome::Created(config) => config,
        crate::config::LoadOutcome::Failed(_) => return,
    };
    // 読み込んだものをそのまま保存し直す (内容は変えない)
    let _ = crate::config::save(&config);
    let start = Instant::now();
    for _ in 0..20 {
        let _ = crate::config::save(&config);
    }
    println!(
        "config::save {:>8.3} ms/回 (お気に入り登録のたびに UI スレッドで同期実行)",
        start.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
}

/// 起動シーケンスの内訳。`--selftest` は完了まで実測 1035ms かかっており、
/// `Index::build` (91ms) だけでは説明がつかない。どこで時間を使っているか。
///
/// トレイアイコンは設定読込とホットキー登録を終えてから出す設計なので
/// (`main.rs` のコメント参照)、ここが長いとアイコンが出るまで待たされる。
///
/// 実測の内訳 (初回):
///
/// | 段 | 実測 |
/// |---|---|
/// | menu::build (1 回目) | 90.63 ms |
/// | menu::build (2 回目) | 0.81 ms |
/// | Index::build | 58.95 ms |
/// | dynamic::refresh | 54.77 ms |
/// | apps::scan | 33.32 ms |
/// | browser_history::scan | 3.84 ms |
/// | config::load | 2.84 ms |
///
/// `menu::build` の 100 倍差はアイコン解決 (`SHGetFileInfo`) がキャッシュに
/// 乗るかどうかで決まる。これを見て `WM_SETTINGCHANGE` の絞り込みを入れた
/// (無関係な設定変更でキャッシュを捨てると 15ms 払う)。
///
/// 起動そのものは短くしない。アイコンは「設定読込とホットキー登録を
/// 終えてから出す」設計で、先に出すとユーザーが効かないアイコンを
/// 触れてしまう (`main.rs` の意図的な順序)。常駐アプリのログイン時
/// 1 秒は体感の対象でもないので、正しさを優先する。
#[test]
#[ignore = "手動計測用"]
fn bench_startup_parts() {
    use std::time::Instant;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    macro_rules! part {
        ($label:expr, $body:expr) => {{
            let start = Instant::now();
            let value = $body;
            println!(
                "{:<34} {:>8.2} ms",
                $label,
                start.elapsed().as_secs_f64() * 1000.0
            );
            value
        }};
    }
    // 初回 (キャッシュが温まっていない状態) を見たいので 1 回ずつ
    let config = part!(
        "config::load",
        match crate::config::load() {
            crate::config::LoadOutcome::Loaded(c) | crate::config::LoadOutcome::Created(c) => c,
            crate::config::LoadOutcome::Failed(_) => return,
        }
    );
    let dynamic = part!("dynamic::refresh", crate::dynamic::refresh());
    part!("menu::build (1 回目)", {
        let _ = crate::menu::build(&config, &dynamic);
    });
    // アイコンキャッシュが温まった状態。差が大きければアイコン解決が支配的
    part!("menu::build (2 回目)", {
        let _ = crate::menu::build(&config, &dynamic);
    });
    part!("icon::clear_cache 後の menu::build", {
        crate::icon::clear_cache();
        let _ = crate::menu::build(&config, &dynamic);
    });
    part!("Index::build", {
        let _ = Index::build(&config, &dynamic);
    });
    part!("bookmarks::scan", {
        let _ = crate::bookmarks::scan();
    });
    part!("browser_history::scan", {
        let _ = crate::browser_history::scan();
    });
    part!("apps::scan", {
        let _ = crate::apps::scan();
    });
    part!("Ranking::load", {
        let _ = crate::quick_launch_history::Ranking::load();
    });
}

/// メニューを開くたびに走る `WM_MEASUREITEM` の採寸。
///
/// `menu_draw::text_extent` は項目 1 件ごとに `GetDC` →
/// `SelectObject` → `GetTextExtentPoint32W` → `ReleaseDC` を回す。
/// 実機は 48 項目あり、これがトリガー経路 (表示予算 50ms) に乗る。
#[test]
#[ignore = "手動計測用"]
fn bench_menu_measure() {
    use std::time::Instant;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{GetDC, GetTextExtentPoint32W, ReleaseDC};

    let labels: Vec<String> = (0..48)
        .map(|i| format!("Project Folder {i}  [main]"))
        .collect();

    // 現行と同じ形: 項目ごとに GetDC / ReleaseDC
    let start = Instant::now();
    for _ in 0..50 {
        let mut total = 0i32;
        for label in &labels {
            let wide: Vec<u16> = label.encode_utf16().collect();
            unsafe {
                let hdc = GetDC(None);
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
                ReleaseDC(None, hdc);
                total += size.cx;
            }
        }
        std::hint::black_box(total);
    }
    let per_item_dc = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

    // 比較: DC を 1 回だけ取って使い回す
    let start = Instant::now();
    for _ in 0..50 {
        let mut total = 0i32;
        unsafe {
            let hdc = GetDC(None);
            for label in &labels {
                let wide: Vec<u16> = label.encode_utf16().collect();
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
                total += size.cx;
            }
            ReleaseDC(None, hdc);
        }
        std::hint::black_box(total);
    }
    let shared_dc = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

    // 実装 (キャッシュ付き `menu_draw::text_extent`) を通した場合
    crate::menu_draw::clear_for_bench();
    let start = Instant::now();
    for label in &labels {
        std::hint::black_box(crate::menu_draw::text_extent_for_bench(label));
    }
    let cold = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    for _ in 0..50 {
        for label in &labels {
            std::hint::black_box(crate::menu_draw::text_extent_for_bench(label));
        }
    }
    let warm = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

    println!(
        "48 項目の採寸: 項目ごとに GetDC {per_item_dc:>8.4} ms  /  DC 使い回し {shared_dc:>8.4} ms"
    );
    println!("  実装 (キャッシュ付き): 1 回目 {cold:>8.4} ms  /  2 回目以降 {warm:>8.4} ms");
}

/// メニューの `WM_DRAWITEM` で項目ごとに走るブラシ生成・破棄。
/// `fill_background` が 1 項目につき `CreateSolidBrush` → `FillRect` →
/// `DeleteObject` を回す。実機は 48 項目。
///
/// 結論: 48 項目で 0.056ms、色ごとに使い回しても 0.036ms にしかならない
/// (差 0.02ms)。`CreateSolidBrush` は `GetDC` (同じ 48 回で 0.73ms、
/// `bench_menu_measure` 参照) より 1 桁以上安い。「GDI オブジェクトの
/// 生成・破棄は重い」と一括りにせず、種類ごとに測ること。
/// この差では使い回しの複雑さに見合わないので手を入れない。
#[test]
#[ignore = "手動計測用"]
fn bench_menu_fill_brush() {
    use std::time::Instant;
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, FillRect, GetDC, HGDIOBJ,
        ReleaseDC,
    };

    let rect = RECT {
        left: 0,
        top: 0,
        right: 240,
        bottom: 24,
    };
    // 実際の描画先に近いメモリ DC で測る
    unsafe {
        let screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(screen));

        // 現行: 項目ごとに作って捨てる
        let start = Instant::now();
        for _ in 0..50 {
            for i in 0..48 {
                let color = COLORREF(if i % 8 == 0 { 0x0041_4141 } else { 0x0020_2020 });
                let brush = CreateSolidBrush(color);
                FillRect(hdc, &rect, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
        }
        let per_item = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

        // 比較: 色ごとに 1 本だけ作って使い回す (2 色)
        let normal = CreateSolidBrush(COLORREF(0x0020_2020));
        let hot = CreateSolidBrush(COLORREF(0x0041_4141));
        let start = Instant::now();
        for _ in 0..50 {
            for i in 0..48 {
                let brush = if i % 8 == 0 { hot } else { normal };
                FillRect(hdc, &rect, brush);
            }
        }
        let cached = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
        let _ = DeleteObject(HGDIOBJ(normal.0));
        let _ = DeleteObject(HGDIOBJ(hot.0));

        let _ = DeleteDC(hdc);
        ReleaseDC(None, screen);
        println!(
            "48 項目の地塗り: 項目ごとに生成 {per_item:>8.4} ms  /  色ごとに使い回し {cached:>8.4} ms"
        );
    }
}

/// Quick Launch の行描画で走る GDI オブジェクト生成。
/// `draw_list_item` は 1 行につき最大 4 個 (地・選択カードのブラシとペン・
/// アクセント) を作って捨てる。可視 24 行ぶんで再描画 1 回。
///
/// 結論: 再描画 1 回で 0.0272ms、使い回しても 0.0216ms (差 0.006ms)。
/// 表示予算 50ms の 0.01% でしかない。`perf/quick-launch-gdi-cache`
/// ブランチで使い回しを実装したが、162 行のモジュールと色キー表を
/// 足す価値が無いので採用しない (`bench_menu_fill_brush` と同じ結論)。
#[test]
#[ignore = "手動計測用"]
fn bench_quick_launch_row_brushes() {
    use std::time::Instant;
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, FillRect, GetDC,
        HGDIOBJ, PS_SOLID, ReleaseDC,
    };
    const ROWS: usize = 24;

    let rect = RECT {
        left: 0,
        top: 0,
        right: 560,
        bottom: 40,
    };
    unsafe {
        let screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(screen));

        // 現行: 行ごとに作って捨てる (地 1 + 選択行だけ 3)
        let start = Instant::now();
        for _ in 0..50 {
            for row in 0..ROWS {
                let background = CreateSolidBrush(COLORREF(0x000D_0D0D));
                FillRect(hdc, &rect, background);
                let _ = DeleteObject(HGDIOBJ(background.0));
                if row == 0 {
                    let card = CreateSolidBrush(COLORREF(0x002C_2926));
                    let pen = CreatePen(PS_SOLID, 1, COLORREF(0x006E_5A3A));
                    let accent = CreateSolidBrush(COLORREF(0x006F_A8C9));
                    FillRect(hdc, &rect, card);
                    let _ = DeleteObject(HGDIOBJ(card.0));
                    let _ = DeleteObject(HGDIOBJ(pen.0));
                    let _ = DeleteObject(HGDIOBJ(accent.0));
                }
            }
        }
        let per_row = start.elapsed().as_secs_f64() * 1000.0 / 50.0;

        // 使い回し
        let background = CreateSolidBrush(COLORREF(0x000D_0D0D));
        let card = CreateSolidBrush(COLORREF(0x002C_2926));
        let pen = CreatePen(PS_SOLID, 1, COLORREF(0x006E_5A3A));
        let accent = CreateSolidBrush(COLORREF(0x006F_A8C9));
        let start = Instant::now();
        for _ in 0..50 {
            for row in 0..ROWS {
                FillRect(hdc, &rect, background);
                if row == 0 {
                    FillRect(hdc, &rect, card);
                }
            }
        }
        let cached = start.elapsed().as_secs_f64() * 1000.0 / 50.0;
        for obj in [background.0, card.0, accent.0] {
            let _ = DeleteObject(HGDIOBJ(obj));
        }
        let _ = DeleteObject(HGDIOBJ(pen.0));

        let _ = DeleteDC(hdc);
        ReleaseDC(None, screen);
        println!(
            "{ROWS} 行の再描画 1 回: 行ごとに生成 {per_row:>8.4} ms  /  使い回し {cached:>8.4} ms"
        );
    }
}
