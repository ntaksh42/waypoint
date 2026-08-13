//! 設定の項目からネイティブのポップアップメニューを組み立てる。
//!
//! 表示は `TrackPopupMenuEx`。WPF 等のウィンドウを使わないので
//! 描画も挙動もシステム標準のまま得られる。

use std::collections::BTreeMap;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MENU_ITEM_FLAGS, MF_DISABLED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MFT_STRING, SetForegroundWindow, TPM_LEFTALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenuEx,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::config::{Config, Item, OpenMode};

/// メニュー項目が選ばれたときに実行する内容。
/// メニュー ID (usize) からこれを引く。
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// 展開済みの絶対パス。
    pub path: String,
    pub open: OpenMode,
}

/// 構築したメニューと、ID → 動作の対応表。
pub struct BuiltMenu {
    menu: HMENU,
    actions: BTreeMap<usize, Action>,
}

impl BuiltMenu {
    /// 選択された ID に対応する動作を返す。
    pub fn action(&self, id: usize) -> Option<&Action> {
        self.actions.get(&id)
    }

    /// 登録された動作の数。テストと診断用。
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// カーソル位置にメニューを表示し、選ばれた動作を返す。
    ///
    /// `owner` は必ず事前に前面化する。そうしないとメニュー外を
    /// クリックしても閉じない (R-2) 。
    pub fn track(&self, owner: HWND, at: POINT) -> Option<&Action> {
        let id = unsafe {
            // これを呼ばないとメニューが閉じなくなる
            let _ = SetForegroundWindow(owner);
            TrackPopupMenuEx(
                self.menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN).0,
                at.x,
                at.y,
                owner,
                None,
            )
        };
        if id.0 == 0 {
            return None; // Esc または領域外クリックで取り消し
        }
        self.action(id.0 as usize)
    }
}

impl Drop for BuiltMenu {
    fn drop(&mut self) {
        // サブメニューは親の DestroyMenu で連鎖的に破棄される
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

/// 設定からメニューを構築する。
///
/// 変数展開は構築時に一度だけ行う (FR-5.3) 。解決できない項目は
/// グレー表示にし、選んでも何も起きないようにする (FR-2.6 / FR-5.4) 。
pub fn build(cfg: &Config) -> Result<BuiltMenu> {
    let mut ctx = BuildCtx {
        vars: &cfg.variables,
        numeric: cfg.settings.menu.numeric_accelerators,
        next_id: FIRST_ITEM_ID,
        actions: BTreeMap::new(),
    };
    let menu = unsafe { build_level(&cfg.items, &mut ctx)? };
    Ok(BuiltMenu {
        menu,
        actions: ctx.actions,
    })
}

/// 0 は「取り消し」を表すため、項目 ID は 1 から始める。
const FIRST_ITEM_ID: usize = 1;

struct BuildCtx<'a> {
    vars: &'a BTreeMap<String, String>,
    numeric: bool,
    next_id: usize,
    actions: BTreeMap<usize, Action>,
}

unsafe fn build_level(items: &[Item], ctx: &mut BuildCtx) -> Result<HMENU> {
    unsafe {
        let menu = CreatePopupMenu()?;
        // 各階層の先頭 9 件に 1〜9 を割り当てる (FR-2.4)
        let mut accel = 0usize;

        for item in items {
            match item {
                Item::Separator { name: None } => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                }
                // 名前つき区切りは見出し。選択不可にする
                Item::Separator { name: Some(text) } => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                    let label = HSTRING::from(format!("— {text} —"));
                    AppendMenuW(
                        menu,
                        MF_STRING | MF_DISABLED | MF_GRAYED,
                        0,
                        PCWSTR(label.as_ptr()),
                    )?;
                }
                Item::Submenu { name, items } => {
                    let sub = build_level(items, ctx)?;
                    accel += 1;
                    let label = HSTRING::from(decorate(name, ctx.numeric, accel));
                    AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(label.as_ptr()))?;
                }
                Item::Folder {
                    name, path, open, ..
                } => {
                    accel += 1;
                    let resolved = crate::config::expand(path, ctx.vars);
                    append_leaf(menu, ctx, name, resolved, open.unwrap_or_default(), accel)?;
                }
                Item::SpecialFolder {
                    name,
                    known_folder,
                    open,
                } => {
                    accel += 1;
                    let resolved = crate::known_folder::resolve(known_folder);
                    append_leaf(menu, ctx, name, resolved, open.unwrap_or_default(), accel)?;
                }
            }
        }
        Ok(menu)
    }
}

/// 葉 (フォルダ / 特殊フォルダ) を追加する。
/// パスが解決できなければグレー表示にして ID を振らない。
unsafe fn append_leaf(
    menu: HMENU,
    ctx: &mut BuildCtx,
    name: &str,
    resolved: Option<String>,
    open: OpenMode,
    accel: usize,
) -> Result<()> {
    unsafe {
        let label = HSTRING::from(decorate(name, ctx.numeric, accel));
        match resolved {
            Some(path) => {
                let id = ctx.next_id;
                ctx.next_id += 1;
                ctx.actions.insert(id, Action { path, open });
                AppendMenuW(menu, MF_STRING, id, PCWSTR(label.as_ptr()))?;
            }
            None => {
                // 解決できない項目は選べないようにする (FR-2.6)
                AppendMenuW(
                    menu,
                    MF_STRING | MF_DISABLED | MF_GRAYED,
                    0,
                    PCWSTR(label.as_ptr()),
                )?;
            }
        }
        Ok(())
    }
}

/// 上位 9 件に `&1 ` のようなアクセラレータを前置する (FR-2.4) 。
fn decorate(name: &str, numeric: bool, accel: usize) -> String {
    if numeric && (1..=9).contains(&accel) {
        format!("&{accel}  {name}")
    } else {
        // 項目名の & はリテラルの & として出すためエスケープする
        name.replace('&', "&&")
    }
}

// MENU_ITEM_FLAGS の | を使うために型を揃える
const _: () = {
    let _ = MFT_STRING;
    let _: MENU_ITEM_FLAGS = MF_STRING;
};
