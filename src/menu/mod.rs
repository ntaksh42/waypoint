//! 設定の項目からネイティブのポップアップメニューを組み立てる。
//!
//! 表示は `TrackPopupMenuEx`。WPF 等のウィンドウを使わないので
//! 描画も挙動もシステム標準のまま得られる。

mod build;
mod label;

use std::collections::BTreeMap;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyMenu, HMENU, SetForegroundWindow, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TPM_TOPALIGN, TrackPopupMenuEx,
};
use windows::core::Result;

use crate::config::{Config, OpenMode};
use crate::dynamic::Menus as DynamicMenus;

use build::{append_footer, append_in_the_works, build_level};

/// メニュー項目が選ばれたときに実行する内容。
/// メニュー ID (usize) からこれを引く。
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Open {
        /// 展開済みの絶対パス。
        path: String,
        open: OpenMode,
    },
    ActivateWindow {
        hwnd: isize,
    },
    OpenShell {
        target: String,
    },
}

/// ランチャーメニューで選ばれた操作。
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Action(Action),
    Settings,
    Reload,
    Close,
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

    /// 全項目を (ID, 開き方, 解決済みパス) で返す。診断用。
    pub fn dump(&self) -> Vec<(usize, String, String)> {
        self.actions
            .iter()
            .map(|(id, a)| match a {
                Action::Open { path, open } => {
                    let mode = match open {
                        OpenMode::NewWindow => "newWindow",
                        OpenMode::Reuse => "reuse",
                    };
                    (*id, mode.to_string(), path.clone())
                }
                Action::ActivateWindow { hwnd } => {
                    (*id, "activateWindow".to_string(), hwnd.to_string())
                }
                Action::OpenShell { target } => (*id, "openShell".to_string(), target.clone()),
            })
            .collect()
    }

    /// カーソル位置にメニューを表示し、選ばれた動作を返す。
    ///
    /// `owner` は必ず事前に前面化する。そうしないとメニュー外を
    /// クリックしても閉じない (R-2) 。
    pub fn track(&self, owner: HWND, at: POINT) -> Option<Selection> {
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
        match id.0 as usize {
            ID_SETTINGS => Some(Selection::Settings),
            ID_RELOAD => Some(Selection::Reload),
            ID_CLOSE => Some(Selection::Close),
            id => self.action(id).cloned().map(Selection::Action),
        }
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
pub fn build(cfg: &Config, dynamic: &DynamicMenus) -> Result<BuiltMenu> {
    // アイコン取得より前に反映する。寸法が変わればキャッシュも捨てられる
    crate::icon::set_icon_size(cfg.settings.menu.icon_size);
    // 前回の描画内容は使わない。ID を振り直すので必ず捨てる
    crate::menu_draw::clear();
    let mut ctx = BuildCtx {
        vars: &cfg.variables,
        numeric: cfg.settings.menu.numeric_accelerators,
        next_id: FIRST_ITEM_ID,
        actions: BTreeMap::new(),
    };
    let menu = unsafe { build_level(&cfg.items, false, &mut ctx)? };
    unsafe {
        append_in_the_works(menu, dynamic, &mut ctx)?;
        append_footer(menu)?;
    }
    Ok(BuiltMenu {
        menu,
        actions: ctx.actions,
    })
}

/// 0 は「取り消し」を表すため、項目 ID は 1 から始める。
const FIRST_ITEM_ID: usize = 1;
pub(crate) const ID_SETTINGS: usize = 0xe001;
pub(crate) const ID_RELOAD: usize = 0xe002;
pub(crate) const ID_CLOSE: usize = 0xe003;
pub(crate) const ICON_RELOAD: &[u8] = include_bytes!("../../assets/menu/reload.png");
pub(crate) const ICON_CLOSE: &[u8] = include_bytes!("../../assets/menu/close.png");
pub(crate) const ICON_WINDOW: &[u8] = include_bytes!("../../assets/menu/window.png");

pub(crate) struct BuildCtx<'a> {
    vars: &'a BTreeMap<String, String>,
    numeric: bool,
    next_id: usize,
    actions: BTreeMap<usize, Action>,
}
