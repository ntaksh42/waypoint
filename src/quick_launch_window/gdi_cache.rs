//! 固定色の GDI オブジェクト (ブラシ・ペン) の使い回し。
//!
//! 描画経路は 1 回の再描画で行数ぶん走る (`draw_list_item` は
//! `WM_DRAWITEM` ごと = 候補 1 行ごとに呼ばれる)。そこで毎回
//! `CreateSolidBrush` → `DeleteObject` すると、色が定数であっても
//! 行数 x 再描画回数ぶんの GDI ハンドル生成・破棄が走る。
//!
//! 色が `mod.rs` のコンパイル時定数であるものだけをここで保持する。
//! テーマや DPI で変わる色は対象外 (フォントが DPI をキーに作り直して
//! いるのと同じ理由で、キャッシュの無効化条件を増やしたくない)。
//!
//! スレッドローカルに置き、プロセス終了まで解放しない。UI スレッドは
//! 常駐部と同じ寿命で、ウィンドウを閉じても次に開き直すため、
//! 閉じるたびに捨てても作り直すだけになる。プロセス終了時の GDI
//! ハンドルは OS が回収する。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{CreatePen, CreateSolidBrush, HBRUSH, HPEN, PS_SOLID};

/// 色ごとのブラシ。`Cell` に生ハンドルの整数値を持つ (HBRUSH は
/// `Copy` だが `Default` を持たないため、0 を「未作成」として扱う)。
struct BrushSlot(Cell<isize>);

impl BrushSlot {
    const fn new() -> Self {
        Self(Cell::new(0))
    }

    fn get(&self, color: COLORREF) -> HBRUSH {
        let cached = self.0.get();
        if cached != 0 {
            return HBRUSH(cached as *mut _);
        }
        let brush = unsafe { CreateSolidBrush(color) };
        self.0.set(brush.0 as isize);
        brush
    }
}

struct PenSlot(Cell<isize>);

impl PenSlot {
    const fn new() -> Self {
        Self(Cell::new(0))
    }

    fn get(&self, color: COLORREF) -> HPEN {
        let cached = self.0.get();
        if cached != 0 {
            return HPEN(cached as *mut _);
        }
        let pen = unsafe { CreatePen(PS_SOLID, 1, color) };
        self.0.set(pen.0 as isize);
        pen
    }
}

thread_local! {
    static BACKGROUND_BRUSH: BrushSlot = const { BrushSlot::new() };
    static SURFACE_HOVER_BRUSH: BrushSlot = const { BrushSlot::new() };
    static SURFACE_PEN: PenSlot = const { PenSlot::new() };
    static ACCENT_PEN: PenSlot = const { PenSlot::new() };
    static CARD_PEN: PenSlot = const { PenSlot::new() };
}

/// 一覧の地の色。`draw_list_item` が行ごとに使う。
pub(super) fn background_brush() -> HBRUSH {
    BACKGROUND_BRUSH.with(|slot| slot.get(super::BACKGROUND))
}

/// 選択カード・区分見出しの区切り線・バッジの地。
pub(super) fn surface_hover_brush() -> HBRUSH {
    SURFACE_HOVER_BRUSH.with(|slot| slot.get(super::SURFACE_HOVER))
}

/// 検索窓の外枠。
pub(super) fn surface_pen() -> HPEN {
    SURFACE_PEN.with(|slot| slot.get(super::SURFACE))
}

/// バッジの輪郭。
pub(super) fn accent_pen() -> HPEN {
    ACCENT_PEN.with(|slot| slot.get(super::ACCENT))
}

/// 選択カードの輪郭。地の `SURFACE_HOVER` より一段明るい固定色。
pub(super) fn card_pen() -> HPEN {
    CARD_PEN.with(|slot| slot.get(super::rgb(58, 90, 110)))
}

/// 色をキーにした小さなブラシ／ペンの表。
///
/// アイコンの地 (`draw_icon_backdrop`) と選択行のアクセント線は、行ごとに
/// 項目の種類で色が変わるため固定スロットには置けない。ただし取り得る色は
/// `badge.rs` の定数から来る 10 色程度に限られるので、色をキーに引く線形
/// 探索の表で足りる (件数が少ないうちは `HashMap` より速く、確保も無い)。
///
/// 想定より色が増えた場合の歯止め。取り得る色は `badge.rs` の定数由来で
/// 10 色程度なので通常は到達しない。到達したらそれ以上は覚えず、既に
/// 覚えた中の先頭を使い回す (返り値を破棄しない規約を保つため、
/// 都度生成して返すことはしない)。
const MAX_CACHED_COLORS: usize = 16;

struct ColorTable {
    /// (色, ハンドルの生値)
    brushes: RefCell<Vec<(u32, isize)>>,
    pens: RefCell<Vec<(u32, isize)>>,
}

impl ColorTable {
    const fn new() -> Self {
        Self {
            brushes: RefCell::new(Vec::new()),
            pens: RefCell::new(Vec::new()),
        }
    }

    fn brush(&self, color: COLORREF) -> HBRUSH {
        let mut table = self.brushes.borrow_mut();
        if let Some((_, handle)) = table.iter().find(|(key, _)| *key == color.0) {
            return HBRUSH(*handle as *mut _);
        }
        if table.len() >= MAX_CACHED_COLORS {
            // 到達しない想定。到達しても漏らさないことを優先する。
            return HBRUSH(table[0].1 as *mut _);
        }
        let brush = unsafe { CreateSolidBrush(color) };
        table.push((color.0, brush.0 as isize));
        brush
    }

    fn pen(&self, color: COLORREF) -> HPEN {
        let mut table = self.pens.borrow_mut();
        if let Some((_, handle)) = table.iter().find(|(key, _)| *key == color.0) {
            return HPEN(*handle as *mut _);
        }
        if table.len() >= MAX_CACHED_COLORS {
            return HPEN(table[0].1 as *mut _);
        }
        let pen = unsafe { CreatePen(PS_SOLID, 1, color) };
        table.push((color.0, pen.0 as isize));
        pen
    }
}

thread_local! {
    static BY_COLOR: ColorTable = const { ColorTable::new() };
}

/// 色を指定してブラシを得る。表に無ければ作って覚える。
///
/// 返り値は表が所有する。**呼び出し側は破棄してはならない。**
pub(super) fn brush_for(color: COLORREF) -> HBRUSH {
    BY_COLOR.with(|table| table.brush(color))
}

/// 色を指定してペンを得る。規約は `brush_for` と同じ。
pub(super) fn pen_for(color: COLORREF) -> HPEN {
    BY_COLOR.with(|table| table.pen(color))
}
