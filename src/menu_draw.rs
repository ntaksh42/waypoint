//! オーナードローによるメニュー項目の描画 (FR-2.3) 。
//!
//! `TrackPopupMenuEx` は `MIIM_BITMAP` で渡したアイコンを行の高さに
//! 反映しない。実測では 32px のアイコンを付けても行はシステムの
//! メニュー行高 (`SM_CYMENU`) のままで、アイコンだけが大きくなる。
//!
//! 行の高さを制御する手段はオーナードローしかないので、
//! `MF_OWNERDRAW` の項目として自前で測って描く。
//! フォントはシステムのメニューフォントをそのまま使い、
//! 文字の大きさは変えない (高さと余白だけ広げる) 。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::{COLORREF, LPARAM, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_MENU, COLOR_MENUTEXT,
    CreateFontIndirectW, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, FillRect, GetSysColor, GetTextExtentPoint32W, HBITMAP,
    HDC, HFONT, HGDIOBJ, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    CloseThemeData, DRAWITEMSTRUCT, GetThemeColor, HTHEME, MEASUREITEMSTRUCT, MENU_POPUPITEM,
    MPI_DISABLED, MPI_HOT, MPI_NORMAL, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, OpenThemeData,
    POPUPITEMSTATES, TMT_FILLCOLOR, TMT_TEXTCOLOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::w;

/// アイコンとテキストの間、および左右の余白。
const PADDING: i32 = 8;
/// 行の上下に足す余白。QAP のメニューに近い間隔にする。
const ROW_PADDING: i32 = 6;
/// サブメニュー矢印に確保する幅。
const ARROW_WIDTH: i32 = 16;
/// 区切り線の行の高さ。
const SEPARATOR_HEIGHT: i32 = 9;
/// ダーク表示での区切り線の色。地 (`DARK_BACKGROUND`) より少し明るい。
const DARK_SEPARATOR: u32 = 0x00454545;

thread_local! {
    /// オーナードロー項目の描画内容。`itemData` に入れた ID から引く。
    static ITEMS: RefCell<HashMap<usize, OwnerDrawItem>> = RefCell::new(HashMap::new());
    /// メニューフォント。`SPI_GETNONCLIENTMETRICS` から一度だけ作る。
    static MENU_FONT: RefCell<Option<isize>> = const { RefCell::new(None) };
    /// メニューのビジュアルスタイル。ダーク表示の色はここから取る。
    static MENU_THEME: RefCell<Option<isize>> = const { RefCell::new(None) };
    /// ラベルごとの採寸結果。`WM_MEASUREITEM` はメニューを開くたびに
    /// 全項目ぶん飛ぶが、ラベルもフォントも開くたびには変わらない。
    /// フォントを作り直すとき (`reset_font`) に一緒に捨てる。
    static TEXT_EXTENTS: RefCell<HashMap<String, (i32, i32)>> =
        RefCell::new(HashMap::new());
}

/// メニューのテーマハンドル。
///
/// 色を `GetSysColor` から取るとライトテーマの値になり、
/// `theme::enable_dark_menus()` の指定を無視してしまう (FR-2.7) 。
/// uxtheme のメニューテーマから引けば、ダーク指定がそのまま反映される。
fn menu_theme() -> Option<HTHEME> {
    MENU_THEME.with(|cached| {
        if let Some(raw) = *cached.borrow() {
            return (raw != 0).then_some(HTHEME(raw));
        }
        let theme = unsafe { OpenThemeData(None, w!("Menu")) };
        let theme = (!theme.is_invalid()).then_some(theme);
        *cached.borrow_mut() = Some(theme.map_or(0, |t: HTHEME| t.0));
        theme
    })
}

/// 1 項目分の描画内容。
#[derive(Clone)]
pub struct OwnerDrawItem {
    /// 表示するラベル。アクセラレータの `&` は取り除いてある。
    pub text: String,
    /// 左端に描くアイコン。無ければ空ける。
    pub bitmap: Option<isize>,
    /// サブメニューなら右端に矢印を描く。
    pub submenu: bool,
    /// 選択できない項目 (解決できないパス・見出し) 。
    pub disabled: bool,
    /// 区切り線。文字を持たず、細い横線だけを描く。
    pub separator: bool,
}

impl OwnerDrawItem {
    /// 区切り線の描画内容。
    pub fn separator() -> Self {
        Self {
            text: String::new(),
            bitmap: None,
            submenu: false,
            disabled: true,
            separator: true,
        }
    }
}

/// 描画内容を登録し、`itemData` へ渡す ID を返す。
pub fn register(item: OwnerDrawItem) -> usize {
    ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        // 0 は「登録なし」と区別したいので 1 から振る
        let id = items.len() + 1;
        items.insert(id, item);
        id
    })
}

/// 登録済みの描画内容を捨てる。メニュー再構築のたびに呼ぶ。
///
/// 採寸キャッシュ (`TEXT_EXTENTS`) もここで捨てる。ラベルには開いている
/// ウィンドウのタイトルや Recent Folders が入り、再構築のたびに変わる。
/// 捨てないと常駐プロセスで際限なく育つ。
pub fn clear() {
    ITEMS.with(|items| items.borrow_mut().clear());
    TEXT_EXTENTS.with(|cache| cache.borrow_mut().clear());
}

fn lookup(id: usize) -> Option<OwnerDrawItem> {
    ITEMS.with(|items| items.borrow().get(&id).cloned())
}

/// システムのメニューフォント。文字サイズは変えないのでそのまま使う。
fn menu_font() -> Option<HFONT> {
    MENU_FONT.with(|cached| {
        if let Some(raw) = *cached.borrow() {
            return (raw != 0).then_some(HFONT(raw as *mut _));
        }
        let font = unsafe {
            let mut metrics = NONCLIENTMETRICSW {
                cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
                ..Default::default()
            };
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                size_of::<NONCLIENTMETRICSW>() as u32,
                Some(&mut metrics as *mut _ as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok()
            .and_then(|()| CreateFontIndirectW(&metrics.lfMenuFont).into())
        };
        *cached.borrow_mut() = Some(font.map_or(0, |f: HFONT| f.0 as isize));
        font
    })
}

/// `WM_MEASUREITEM` の応答。行の幅と高さを決める。
///
/// 高さはアイコンと文字の高いほうに余白を足した値。
/// これがそのまま行の高さになる。
pub fn measure(wparam: WPARAM, lparam: LPARAM) -> bool {
    let _ = wparam;
    let measure = lparam.0 as *mut MEASUREITEMSTRUCT;
    if measure.is_null() {
        return false;
    }
    unsafe {
        let Some(item) = lookup((*measure).itemData) else {
            return false;
        };
        if item.separator {
            // 線 1 本ぶんの高さ。項目より薄くする
            (*measure).itemHeight = SEPARATOR_HEIGHT as u32;
            (*measure).itemWidth = 0;
            return true;
        }
        let icon = crate::icon::icon_extent();
        let text = text_extent(&item.text);
        (*measure).itemHeight = (icon.cy.max(text.cy) + ROW_PADDING * 2).max(1) as u32;
        (*measure).itemWidth =
            (icon.cx + PADDING * 3 + text.cx + if item.submenu { ARROW_WIDTH } else { 0 }) as u32;
    }
    true
}

/// メニューフォントで測ったテキストの寸法。
///
/// 結果はラベルごとにキャッシュする。`WM_MEASUREITEM` はメニューを開く
/// たびに全項目ぶん飛び、1 件ごとに `GetDC` / `ReleaseDC` を回す。実機の
/// 48 項目で 0.73ms、キャッシュに乗れば 0.0011ms (実測)。トリガーから
/// 表示までの予算 50ms に乗る経路なので、測り直さずに済ませる。
///
/// キャッシュは `clear()` (メニュー再構築時) で捨てるため、開くたびに
/// 全部が温まっているわけではない。効くのは 1 回の表示の中で同じラベルが
/// 複数回測られる分 (サブメニューと親で同じ項目が出る、同名フォルダが
/// 並ぶ、など) と、再構築を挟まずに開き直した場合。
fn text_extent(text: &str) -> SIZE {
    if let Some((cx, cy)) = TEXT_EXTENTS.with(|cache| cache.borrow().get(text).copied()) {
        return SIZE { cx, cy };
    }
    let size = measure_text(text);
    TEXT_EXTENTS.with(|cache| {
        cache
            .borrow_mut()
            .insert(text.to_string(), (size.cx, size.cy))
    });
    size
}

/// 実際に GDI へ問い合わせる本体。`text_extent` のキャッシュミス時のみ。
fn measure_text(text: &str) -> SIZE {
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::GetDC(None);
        if hdc.is_invalid() {
            return SIZE { cx: 0, cy: 0 };
        }
        let old = menu_font().map(|font| SelectObject(hdc, font.into()));
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        if let Some(old) = old {
            SelectObject(hdc, old);
        }
        windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc);
        size
    }
}

/// `WM_DRAWITEM` の応答。背景・アイコン・文字・矢印を描く。
pub fn draw(wparam: WPARAM, lparam: LPARAM) -> bool {
    let _ = wparam;
    let draw = lparam.0 as *const DRAWITEMSTRUCT;
    if draw.is_null() {
        return false;
    }
    unsafe {
        let draw = &*draw;
        let Some(item) = lookup(draw.itemData) else {
            return false;
        };
        let hdc = draw.hDC;
        let rect = draw.rcItem;
        let selected = draw.itemState.0 & ODS_SELECTED.0 != 0;
        let grayed = draw.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0 || item.disabled;

        // 区切り線は地を塗ってから細い線を 1 本引くだけ
        if item.separator {
            fill_background(hdc, &rect, MPI_NORMAL, false);
            draw_separator(hdc, &rect);
            return true;
        }

        // 項目の状態はテーマの部品状態に対応させる
        let state = if grayed {
            MPI_DISABLED
        } else if selected {
            MPI_HOT
        } else {
            MPI_NORMAL
        };
        fill_background(hdc, &rect, state, selected);

        let icon = crate::icon::icon_extent();
        let icon_x = rect.left + PADDING;
        if let Some(raw) = item.bitmap {
            let y = rect.top + (rect.bottom - rect.top - icon.cy) / 2;
            draw_bitmap(hdc, HBITMAP(raw as *mut _), icon_x, y, icon);
        }

        // 文字色もテーマから取る。GetSysColor だとダーク指定が効かない
        SetTextColor(hdc, text_color(state, grayed, selected));
        SetBkMode(hdc, TRANSPARENT);
        let old = menu_font().map(|font| SelectObject(hdc, font.into()));

        let mut text_rect = RECT {
            left: icon_x + icon.cx + PADDING,
            top: rect.top,
            right: rect.right - if item.submenu { ARROW_WIDTH } else { PADDING },
            bottom: rect.bottom,
        };
        let mut wide: Vec<u16> = item.text.encode_utf16().collect();
        DrawTextW(
            hdc,
            &mut wide,
            &mut text_rect,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );

        if item.submenu {
            draw_arrow(hdc, &rect);
        }

        if let Some(old) = old {
            SelectObject(hdc, old);
        }
    }
    true
}

/// 文字色。テーマから引き、取れない環境ではシステム色へ落とす。
fn text_color(state: POPUPITEMSTATES, grayed: bool, selected: bool) -> COLORREF {
    let themed = menu_theme().and_then(|theme| unsafe {
        GetThemeColor(theme, MENU_POPUPITEM.0, state.0, TMT_TEXTCOLOR).ok()
    });
    themed.unwrap_or_else(|| {
        let fallback = if grayed {
            COLOR_GRAYTEXT
        } else if selected {
            COLOR_HIGHLIGHTTEXT
        } else {
            COLOR_MENUTEXT
        };
        COLORREF(unsafe { GetSysColor(fallback) })
    })
}

/// ダーク表示かどうか。
///
/// テーマは背景色 (`TMT_FILLCOLOR`) を持たないので、明るさの判定には
/// 通常項目の文字色を使う。実測ではダーク時に白 (255,255,255)、
/// ライト時に黒 (0,0,0) が返る。文字が明るいなら地は暗い。
fn is_dark() -> bool {
    let text = menu_theme().and_then(|theme| unsafe {
        GetThemeColor(theme, MENU_POPUPITEM.0, MPI_NORMAL.0, TMT_TEXTCOLOR).ok()
    });
    text.is_some_and(|color| is_light_text(color.0))
}

/// 文字色が明るいか。明るければ地は暗いと判断する。
pub fn is_light_text(color: u32) -> bool {
    let (r, g, b) = (color & 0xff, (color >> 8) & 0xff, (color >> 16) & 0xff);
    r + g + b > 0x180
}

/// ダーク表示時のメニュー地。Windows 11 のメニューに合わせる。
const DARK_BACKGROUND: u32 = 0x002B2B2B;

/// 背景を塗る。
///
/// テーマは `MENU_POPUPBACKGROUND` の塗り色を持たず (実測で
/// `GetThemeColor` が `0x80070490` を返す)、`MPI_NORMAL` の項目も透明。
/// `DrawThemeBackground` に任せると明るい地が描かれ、ダーク指定
/// (FR-2.7) を無視して白く見える。地は自分で塗り、選択時だけ
/// テーマの塗り色を使う。
unsafe fn fill_background(hdc: HDC, rect: &RECT, state: POPUPITEMSTATES, selected: bool) {
    unsafe {
        let dark = is_dark();
        // 選択色はテーマから取れる (ダークで 65,65,65)
        let themed_fill = menu_theme()
            .and_then(|theme| GetThemeColor(theme, MENU_POPUPITEM.0, state.0, TMT_FILLCOLOR).ok());

        let color = match themed_fill {
            Some(color) => color,
            None if dark => COLORREF(DARK_BACKGROUND),
            None => COLORREF(GetSysColor(if selected {
                COLOR_HIGHLIGHT
            } else {
                COLOR_MENU
            })),
        };

        let brush = CreateSolidBrush(color);
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

/// 区切り線を 1 本引く。
///
/// `MF_SEPARATOR` のままだと Windows がシステム色で描き、ダーク表示でも
/// 明るい線が残る (実測で項目だけ黒く、区切りが白く浮いた) 。
unsafe fn draw_separator(hdc: HDC, rect: &RECT) {
    unsafe {
        let color = if is_dark() {
            COLORREF(DARK_SEPARATOR)
        } else {
            COLORREF(GetSysColor(COLOR_GRAYTEXT))
        };
        let y = rect.top + (rect.bottom - rect.top) / 2;
        let line = RECT {
            left: rect.left + PADDING,
            top: y,
            right: rect.right - PADDING,
            bottom: y + 1,
        };
        let brush = CreateSolidBrush(color);
        FillRect(hdc, &line, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

/// サブメニューの矢印。
///
/// `DrawFrameControl` はシステム色で描くためダーク表示だと沈んで見えない。
/// 呼び出し前に設定した文字色で三角を描き、地の色に追従させる。
unsafe fn draw_arrow(hdc: HDC, rect: &RECT) {
    unsafe {
        let mut arrow = RECT {
            left: rect.right - ARROW_WIDTH,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        // Segoe UI Symbol の右向き三角。メニューフォントで出る
        let mut glyph: Vec<u16> = "\u{25B6}".encode_utf16().collect();
        DrawTextW(
            hdc,
            &mut glyph,
            &mut arrow,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
    }
}

/// アルファ付きビットマップを透過して描く。
unsafe fn draw_bitmap(hdc: HDC, bitmap: HBITMAP, x: i32, y: i32, size: SIZE) {
    unsafe {
        use windows::Win32::Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, CreateCompatibleDC, DeleteDC,
        };
        let mem = CreateCompatibleDC(Some(hdc));
        if mem.is_invalid() {
            return;
        }
        let old: HGDIOBJ = SelectObject(mem, bitmap.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(
            hdc, x, y, size.cx, size.cy, mem, 0, 0, size.cx, size.cy, blend,
        );
        SelectObject(mem, old);
        let _ = DeleteDC(mem);
    }
}

/// テーマ変更などでフォントとテーマを作り直す。
///
/// ダーク / ライトの切り替えで色が変わるため、
/// 掴んだままのテーマハンドルを捨てて次回に開き直す。
pub fn reset_font() {
    // フォントが変わると同じラベルでも寸法が変わる。
    // (メニュー再構築を伴わないテーマ変更でもここだけは通る)
    TEXT_EXTENTS.with(|cache| cache.borrow_mut().clear());
    MENU_FONT.with(|cached| {
        if let Some(raw) = cached.borrow_mut().take()
            && raw != 0
        {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(raw as *mut _));
            }
        }
    });
    MENU_THEME.with(|cached| {
        if let Some(raw) = cached.borrow_mut().take()
            && raw != 0
        {
            unsafe {
                let _ = CloseThemeData(HTHEME(raw));
            }
        }
    });
}

/// テスト用。採寸キャッシュの件数。`clear` / `reset_font` で 0 に戻ることを
/// 確かめるためだけに公開する (常駐プロセスで際限なく育たないことの検証)。
#[cfg(test)]
pub fn text_extent_cache_len() -> usize {
    TEXT_EXTENTS.with(|cache| cache.borrow().len())
}

#[cfg(test)]
mod extent_cache_tests {
    use super::{clear, reset_font, text_extent, text_extent_cache_len};

    /// 同じラベルは 2 回目以降キャッシュから返る (件数が増えない)。
    #[test]
    fn same_label_is_measured_once() {
        clear();
        let first = text_extent("Project Folder 1");
        assert_eq!(text_extent_cache_len(), 1);
        let second = text_extent("Project Folder 1");
        assert_eq!(text_extent_cache_len(), 1, "同じラベルで件数が増えている");
        assert_eq!((first.cx, first.cy), (second.cx, second.cy));
    }

    /// 違うラベルはそれぞれ覚える。
    #[test]
    fn distinct_labels_are_cached_separately() {
        clear();
        text_extent("A");
        text_extent("BB");
        text_extent("CCC");
        assert_eq!(text_extent_cache_len(), 3);
    }

    /// メニュー再構築で捨てる。ラベルには開いているウィンドウのタイトルや
    /// Recent Folders が入り再構築のたびに変わるので、捨てないと際限なく育つ。
    #[test]
    fn rebuild_clears_the_cache() {
        clear();
        text_extent("Window title that changes");
        assert_eq!(text_extent_cache_len(), 1);
        clear();
        assert_eq!(text_extent_cache_len(), 0, "再構築で捨てていない");
    }

    /// フォント変更でも捨てる。同じラベルでも寸法が変わるため。
    #[test]
    fn font_reset_clears_the_cache() {
        clear();
        text_extent("Some label");
        assert_eq!(text_extent_cache_len(), 1);
        reset_font();
        assert_eq!(text_extent_cache_len(), 0, "フォント変更で捨てていない");
    }
}

#[cfg(test)]
pub fn text_extent_for_bench(text: &str) -> (i32, i32) {
    let size = text_extent(text);
    (size.cx, size.cy)
}

#[cfg(test)]
pub fn clear_for_bench() {
    clear();
}
