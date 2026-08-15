//! 設定の項目からネイティブのポップアップメニューを組み立てる。
//!
//! 表示は `TrackPopupMenuEx`。WPF 等のウィンドウを使わないので
//! 描画も挙動もシステム標準のまま得られる。

use std::collections::BTreeMap;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::Shell::{
    SHSTOCKICONID, SIID_DOCASSOC, SIID_DOCNOASSOC, SIID_FOLDER, SIID_FOLDEROPEN, SIID_STACK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, HMENU, InsertMenuItemW, MENU_ITEM_FLAGS,
    MENUITEMINFOW, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_STRING, MFS_DISABLED, MFT_OWNERDRAW,
    MFT_SEPARATOR, MIIM_DATA, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_SUBMENU, SetForegroundWindow,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenuEx,
};
use windows::core::Result;

use crate::config::{Config, Item, OpenMode};
use crate::dynamic::{Menus as DynamicMenus, PathEntry, WindowEntry};

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
    let menu = unsafe { build_level(&cfg.items, &mut ctx)? };
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
const ID_SETTINGS: usize = 0xe001;
const ID_RELOAD: usize = 0xe002;
const ID_CLOSE: usize = 0xe003;
const ICON_RELOAD: &[u8] = include_bytes!("../assets/menu/reload.png");
const ICON_CLOSE: &[u8] = include_bytes!("../assets/menu/close.png");
const ICON_WINDOW: &[u8] = include_bytes!("../assets/menu/window.png");

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
                    append_separator(menu)?;
                }
                // 名前つき区切りは見出し。選択不可にする
                Item::Separator { name: Some(text) } => {
                    append_separator(menu)?;
                    append_owner_drawn(
                        menu,
                        MF_STRING | MF_DISABLED | MF_GRAYED,
                        0,
                        &format!("— {text} —"),
                        None,
                        false,
                    )?;
                }
                Item::Submenu { name, items } => {
                    let sub = build_level(items, ctx)?;
                    accel += 1;
                    append_owner_drawn(
                        menu,
                        MF_POPUP,
                        sub.0 as usize,
                        &decorate(name, ctx.numeric, accel),
                        crate::icon::bitmap_for_stock(SIID_FOLDER),
                        true,
                    )?;
                }
                Item::Folder {
                    name,
                    path,
                    open,
                    show_branch,
                    ..
                } => {
                    accel += 1;
                    let resolved = crate::config::expand(path, ctx.vars);
                    // ブランチ名の付与は構築時に済ませる。表示経路では読まない (FR-2.15)
                    let branch = resolved
                        .as_deref()
                        .filter(|_| *show_branch)
                        .and_then(crate::git::branch_of);
                    let label = with_branch(name, branch.as_deref());
                    append_leaf(menu, ctx, &label, resolved, open.unwrap_or_default(), accel)?;
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
                Item::Shell { name, target } => {
                    accel += 1;
                    let id = ctx.next_id;
                    ctx.next_id += 1;
                    append_owner_drawn(
                        menu,
                        MF_STRING,
                        id,
                        &decorate(name, ctx.numeric, accel),
                        crate::icon::bitmap_for_shell(target),
                        false,
                    )?;
                    ctx.actions.insert(
                        id,
                        Action::OpenShell {
                            target: target.clone(),
                        },
                    );
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
        let label = decorate(name, ctx.numeric, accel);
        match resolved {
            Some(path) => {
                let id = ctx.next_id;
                ctx.next_id += 1;
                // アイコンは付かなくても致命的ではないので失敗を無視する
                let bitmap = crate::icon::bitmap_for(&path);
                append_owner_drawn(menu, MF_STRING, id, &label, bitmap, false)?;
                ctx.actions.insert(id, Action::Open { path, open });
            }
            None => {
                // 解決できない項目は選べないようにする (FR-2.6)
                append_owner_drawn(
                    menu,
                    MF_STRING | MF_DISABLED | MF_GRAYED,
                    0,
                    &label,
                    None,
                    false,
                )?;
            }
        }
        Ok(())
    }
}

/// QAP の既定構成に合わせて、動的な作業中メニューを追加する。
unsafe fn append_in_the_works(
    menu: HMENU,
    dynamic: &DynamicMenus,
    ctx: &mut BuildCtx,
) -> Result<()> {
    unsafe {
        append_separator(menu)?;
        let submenu = CreatePopupMenu()?;
        append_path_menu(submenu, "Recent Folders", &dynamic.recent_folders, ctx)?;
        append_path_menu(submenu, "Recent Files", &dynamic.recent_files, ctx)?;
        append_separator(submenu)?;
        append_path_menu(submenu, "Frequent Folders", &dynamic.frequent_folders, ctx)?;
        append_path_menu(submenu, "Frequent Files", &dynamic.frequent_files, ctx)?;
        append_separator(submenu)?;
        append_window_menu(submenu, "Current Windows", &dynamic.current_windows, ctx)?;
        append_separator(submenu)?;
        // Customize も設定画面への入口。Settings... と同じ歯車で揃える
        append_owner_drawn(
            submenu,
            MF_STRING,
            ID_SETTINGS,
            "Customize this menu (or group)",
            crate::icon::bitmap_for_settings(),
            false,
        )?;

        // 作業中の項目が積み重なっている絵。フォルダ系と区別する
        append_owner_drawn(
            menu,
            MF_POPUP,
            submenu.0 as usize,
            "In the Works",
            crate::icon::bitmap_for_stock(SIID_STACK),
            true,
        )?;
        Ok(())
    }
}

unsafe fn append_path_menu(
    parent: HMENU,
    name: &str,
    entries: &[PathEntry],
    ctx: &mut BuildCtx,
) -> Result<()> {
    unsafe {
        let submenu = CreatePopupMenu()?;
        if entries.is_empty() {
            append_owner_drawn(
                submenu,
                MF_STRING | MF_DISABLED | MF_GRAYED,
                0,
                "(Empty)",
                None,
                false,
            )?;
        } else {
            for (index, entry) in entries.iter().enumerate() {
                let id = ctx.next_id;
                ctx.next_id += 1;
                append_owner_drawn(
                    submenu,
                    MF_STRING,
                    id,
                    &decorate(&entry.name, ctx.numeric, index + 1),
                    crate::icon::bitmap_for(&entry.path),
                    false,
                )?;
                ctx.actions.insert(
                    id,
                    Action::Open {
                        path: entry.path.clone(),
                        open: OpenMode::NewWindow,
                    },
                );
            }
        }
        append_close(submenu)?;
        // 先頭項目のアイコンを借りると中身次第で毎回変わり、
        // Recent と Frequent が見分けられない。メニュー種別で固定する
        append_owner_drawn(
            parent,
            MF_POPUP,
            submenu.0 as usize,
            name,
            crate::icon::bitmap_for_stock(path_menu_icon(name)),
            true,
        )?;
        Ok(())
    }
}

unsafe fn append_window_menu(
    parent: HMENU,
    name: &str,
    entries: &[WindowEntry],
    ctx: &mut BuildCtx,
) -> Result<()> {
    unsafe {
        let submenu = CreatePopupMenu()?;
        if entries.is_empty() {
            append_owner_drawn(
                submenu,
                MF_STRING | MF_DISABLED | MF_GRAYED,
                0,
                "(Empty)",
                None,
                false,
            )?;
        } else {
            for (index, entry) in entries.iter().enumerate() {
                let id = ctx.next_id;
                ctx.next_id += 1;
                let bitmap = crate::icon::bitmap_for_window(HWND(entry.hwnd as *mut _))
                    .or_else(|| crate::icon::bitmap_for_asset("window", ICON_WINDOW));
                append_owner_drawn(
                    submenu,
                    MF_STRING,
                    id,
                    &decorate(&entry.title, ctx.numeric, index + 1),
                    bitmap,
                    false,
                )?;
                ctx.actions
                    .insert(id, Action::ActivateWindow { hwnd: entry.hwnd });
            }
        }
        append_close(submenu)?;
        append_owner_drawn(
            parent,
            MF_POPUP,
            submenu.0 as usize,
            name,
            crate::icon::bitmap_for_asset("window", ICON_WINDOW),
            true,
        )?;
        Ok(())
    }
}

unsafe fn append_close(menu: HMENU) -> Result<()> {
    unsafe {
        append_separator(menu)?;
        append_owner_drawn(
            menu,
            MF_STRING,
            ID_CLOSE,
            "Close this menu",
            crate::icon::bitmap_for_asset("close", ICON_CLOSE),
            false,
        )?;
        Ok(())
    }
}

/// QAP と同様に、頻繁に使う管理操作をルートメニューの末尾へ置く。
unsafe fn append_footer(menu: HMENU) -> Result<()> {
    unsafe {
        append_separator(menu)?;
        // Windows の歯車アイコン。自前 PNG は線が細く 16px で潰れていた
        append_owner_drawn(
            menu,
            MF_STRING,
            ID_SETTINGS,
            "Settings...",
            crate::icon::bitmap_for_settings(),
            false,
        )?;
        append_owner_drawn(
            menu,
            MF_STRING,
            ID_RELOAD,
            "Reload config",
            crate::icon::bitmap_for_asset("reload", ICON_RELOAD),
            false,
        )?;

        append_separator(menu)?;
        append_owner_drawn(
            menu,
            MF_STRING,
            ID_CLOSE,
            "Close this menu",
            crate::icon::bitmap_for_asset("close", ICON_CLOSE),
            false,
        )?;
        Ok(())
    }
}

/// オーナードロー項目として追加する。
///
/// `TrackPopupMenuEx` は `MIIM_BITMAP` のアイコンを行の高さに反映しない
/// (実測) 。行の高さを制御するにはオーナードローしかないので、
/// 文字列項目はすべてこの経路で追加する。`itemData` に描画内容の ID を
/// 入れ、`WM_MEASUREITEM` / `WM_DRAWITEM` から引く。
unsafe fn append_owner_drawn(
    menu: HMENU,
    flags: MENU_ITEM_FLAGS,
    id: usize,
    label: &str,
    bitmap: Option<HBITMAP>,
    submenu: bool,
) -> Result<()> {
    unsafe {
        let disabled = flags.0 & (MF_DISABLED.0 | MF_GRAYED.0) != 0;
        let data = crate::menu_draw::register(crate::menu_draw::OwnerDrawItem {
            // アクセラレータの & は描画では出さない
            text: strip_accelerator(label),
            bitmap: bitmap.map(|b| b.0 as isize),
            submenu,
            disabled,
            separator: false,
        });

        // itemData は MIIM_DATA でしか渡せない。AppendMenuW の
        // lpnewitem に入れても文字列として扱われる
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA | MIIM_STATE,
            fType: MFT_OWNERDRAW,
            dwItemData: data,
            ..Default::default()
        };
        if disabled {
            info.fState = MFS_DISABLED;
        }
        if submenu {
            info.fMask |= MIIM_SUBMENU;
            info.hSubMenu = HMENU(id as *mut _);
        } else if id != 0 {
            info.fMask |= MIIM_ID;
            info.wID = id as u32;
        }
        let position = GetMenuItemCount(Some(menu)).max(0) as u32;
        InsertMenuItemW(menu, position, true, &info)?;
        Ok(())
    }
}

/// 区切り線を追加する。
///
/// `MF_SEPARATOR` のままだとシステム色で描かれ、ダーク表示 (FR-2.7) で
/// 白い線が残る。項目と同じくオーナードローにして自前で引く。
unsafe fn append_separator(menu: HMENU) -> Result<()> {
    unsafe {
        let data = crate::menu_draw::register(crate::menu_draw::OwnerDrawItem::separator());
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA | MIIM_STATE,
            // 区切りとしての性質は残したまま描画だけ奪う
            fType: MFT_OWNERDRAW | MFT_SEPARATOR,
            fState: MFS_DISABLED,
            dwItemData: data,
            ..Default::default()
        };
        let position = GetMenuItemCount(Some(menu)).max(0) as u32;
        InsertMenuItemW(menu, position, true, &info)?;
        Ok(())
    }
}

/// `&1  名前` の装飾を描画用の文字列へ直す。
///
/// オーナードローでは `&` を自分で解釈しないので、
/// 単独の `&` は落とし、`&&` はリテラルの `&` に戻す。
fn strip_accelerator(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
        } else if chars.clone().next() == Some('&') {
            chars.next();
            out.push('&');
        }
    }
    out
}

/// Recent / Frequent の各サブメニューに割り当てるアイコン。
///
/// 「最近」は時計 (履歴)、「よく使う」は星 (お気に入り) に相当する
/// Windows 標準アイコンが無いため、フォルダ / ファイルの区別に加えて
/// 開いた・閉じたで最近とよく使うを描き分ける。
fn path_menu_icon(name: &str) -> SHSTOCKICONID {
    match (name.starts_with("Recent"), name.ends_with("Folders")) {
        (true, true) => SIID_FOLDEROPEN,
        (false, true) => SIID_FOLDER,
        (true, false) => SIID_DOCASSOC,
        (false, false) => SIID_DOCNOASSOC,
    }
}

/// ブランチ名があれば項目名の後ろに `[名前]` を付す (FR-2.14) 。
/// リポジトリでない項目は名前のみ。
fn with_branch(name: &str, branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("{name}  [{branch}]"),
        None => name.to_string(),
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

#[cfg(test)]
mod tests {
    use super::{decorate, path_menu_icon, strip_accelerator, with_branch};

    /// Recent / Frequent × フォルダ / ファイルの 4 つが別アイコンになること。
    /// 同じだと In the Works の中で見分けが付かない。
    #[test]
    fn path_menu_icons_are_distinct() {
        let ids: Vec<i32> = [
            "Recent Folders",
            "Frequent Folders",
            "Recent Files",
            "Frequent Files",
        ]
        .iter()
        .map(|name| path_menu_icon(name).0)
        .collect();

        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4, "同じアイコンが割り当てられている: {ids:?}");
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
    fn branch_survives_accelerator_decoration() {
        let label = with_branch("waypoint", Some("feature/x"));
        assert_eq!(decorate(&label, true, 1), "&1  waypoint  [feature/x]");
    }

    /// 項目名の & はエスケープされる。アクセラレータ無効時も同じ規則。
    #[test]
    fn ampersand_in_name_is_escaped_without_accelerator() {
        let label = with_branch("R&D", Some("main"));
        assert_eq!(decorate(&label, false, 1), "R&&D  [main]");
    }

    /// オーナードローでは & を自分で解釈しないので描画前に落とす。
    #[test]
    fn accelerator_marker_is_removed_for_drawing() {
        assert_eq!(strip_accelerator("&1  Downloads"), "1  Downloads");
    }

    /// エスケープされた && はリテラルの & に戻す。
    #[test]
    fn escaped_ampersand_becomes_literal() {
        assert_eq!(strip_accelerator("R&&D"), "R&D");
        assert_eq!(strip_accelerator("&1  R&&D"), "1  R&D");
    }
}
