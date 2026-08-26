//! 起動シーケンスとメニュー描画のベンチ。
//!
//! トリガーから表示までの予算 50ms に乗る経路を測る。ここから
//! `WM_SETTINGCHANGE` の絞り込みと項目ラベルの採寸キャッシュを入れた。
//!
//! 起動そのもの (約 1 秒) は短くしない判断。トレイアイコンは設定読込と
//! ホットキー登録を終えてから出す設計で、先に出すとユーザーが効かない
//! アイコンを触れてしまう (`main.rs` の意図的な順序)。

use super::super::*;

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
