//! 設定の項目からネイティブのポップアップメニューを組み立てる。
//!
//! 表示は `TrackPopupMenuEx`。WPF 等のウィンドウを使わないので
//! 描画も挙動もシステム標準のまま得られる。

use std::collections::BTreeMap;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::Shell::{
    SHSTOCKICONID, SIID_DESKTOPPC, SIID_DOCASSOC, SIID_DOCNOASSOC, SIID_FOLDER, SIID_FOLDEROPEN,
    SIID_STACK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, HMENU, MENUITEMINFOW, MF_DISABLED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP, SetForegroundWindow,
    SetMenuItemInfoW, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    TrackPopupMenuEx,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

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
    AddSpecialFolder,
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
            ID_ADD_SPECIAL_FOLDER => Some(Selection::AddSpecialFolder),
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
    let mut ctx = BuildCtx {
        vars: &cfg.variables,
        numeric: cfg.settings.menu.numeric_accelerators,
        next_id: FIRST_ITEM_ID,
        actions: BTreeMap::new(),
    };
    let menu = unsafe { build_level(&cfg.items, &mut ctx)? };
    unsafe {
        append_in_the_works(menu, dynamic, &mut ctx)?;
        append_special_folders(menu, &mut ctx)?;
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
const ID_ADD_SPECIAL_FOLDER: usize = 0xe004;
const ICON_SETTINGS: &[u8] = include_bytes!("../assets/menu/settings.png");
const ICON_RELOAD: &[u8] = include_bytes!("../assets/menu/reload.png");
const ICON_CLOSE: &[u8] = include_bytes!("../assets/menu/close.png");
const ICON_CUSTOMIZE: &[u8] = include_bytes!("../assets/menu/customize.png");
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
                    if let Some(bitmap) = crate::icon::bitmap_for_stock(SIID_FOLDER) {
                        set_last_item_bitmap(menu, bitmap);
                    }
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
                AppendMenuW(menu, MF_STRING, id, PCWSTR(label.as_ptr()))?;
                // アイコンは付かなくても致命的ではないので失敗を無視する
                if let Some(bmp) = crate::icon::bitmap_for(&path) {
                    set_item_bitmap(menu, id, bmp);
                }
                ctx.actions.insert(id, Action::Open { path, open });
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

/// QAP の既定構成に合わせて、動的な作業中メニューを追加する。
unsafe fn append_in_the_works(
    menu: HMENU,
    dynamic: &DynamicMenus,
    ctx: &mut BuildCtx,
) -> Result<()> {
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        let submenu = CreatePopupMenu()?;
        append_path_menu(submenu, "Recent Folders", &dynamic.recent_folders, ctx)?;
        append_path_menu(submenu, "Recent Files", &dynamic.recent_files, ctx)?;
        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;
        append_path_menu(submenu, "Frequent Folders", &dynamic.frequent_folders, ctx)?;
        append_path_menu(submenu, "Frequent Files", &dynamic.frequent_files, ctx)?;
        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;
        append_window_menu(submenu, "Current Windows", &dynamic.current_windows, ctx)?;
        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(
            submenu,
            MF_STRING,
            ID_SETTINGS,
            w!("Customize this menu (or group)"),
        )?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("customize", ICON_CUSTOMIZE) {
            set_item_bitmap(submenu, ID_SETTINGS, bitmap);
        }

        AppendMenuW(menu, MF_POPUP, submenu.0 as usize, w!("In the Works"))?;
        // 作業中の項目が積み重なっている絵。フォルダ系と区別する
        if let Some(bitmap) = crate::icon::bitmap_for_stock(SIID_STACK) {
            set_last_item_bitmap(menu, bitmap);
        }
        Ok(())
    }
}

/// QAP の My Special Folders と同じ、Windows 標準の場所への入口。
unsafe fn append_special_folders(menu: HMENU, ctx: &mut BuildCtx) -> Result<()> {
    unsafe {
        let submenu = CreatePopupMenu()?;
        AppendMenuW(
            submenu,
            MF_STRING,
            ID_ADD_SPECIAL_FOLDER,
            w!("Add Favorite - Special Folder..."),
        )?;
        // 追加の操作なので開いたフォルダ。閉じたフォルダの項目と見分けられる
        if let Some(bitmap) = crate::icon::bitmap_for_stock(SIID_FOLDEROPEN) {
            set_item_bitmap(submenu, ID_ADD_SPECIAL_FOLDER, bitmap);
        }
        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;

        for (index, (label, known_folder)) in [
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Pictures", "Pictures"),
            ("Downloads", "Downloads"),
        ]
        .into_iter()
        .enumerate()
        {
            append_leaf(
                submenu,
                ctx,
                label,
                crate::known_folder::resolve(known_folder),
                OpenMode::NewWindow,
                index + 1,
            )?;
        }

        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;
        for (index, (label, target)) in [
            ("This PC", "shell:MyComputerFolder"),
            ("Network", "shell:NetworkPlacesFolder"),
            ("All Control Panel Items", "shell:ControlPanelFolder"),
            ("Recycle Bin", "shell:RecycleBinFolder"),
        ]
        .into_iter()
        .enumerate()
        {
            let id = ctx.next_id;
            ctx.next_id += 1;
            let label = HSTRING::from(decorate(label, ctx.numeric, index + 5));
            AppendMenuW(submenu, MF_STRING, id, PCWSTR(label.as_ptr()))?;
            if let Some(bitmap) = crate::icon::bitmap_for_shell(target) {
                set_item_bitmap(submenu, id, bitmap);
            }
            ctx.actions.insert(
                id,
                Action::OpenShell {
                    target: target.to_string(),
                },
            );
        }

        AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(
            submenu,
            MF_STRING,
            ID_SETTINGS,
            w!("Customize this menu (or group)"),
        )?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("customize", ICON_CUSTOMIZE) {
            set_item_bitmap(submenu, ID_SETTINGS, bitmap);
        }
        AppendMenuW(menu, MF_POPUP, submenu.0 as usize, w!("My Special Folders"))?;
        // Windows 標準の場所をまとめた入口なので PC のアイコン
        if let Some(bitmap) = crate::icon::bitmap_for_stock(SIID_DESKTOPPC) {
            set_last_item_bitmap(menu, bitmap);
        }
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
            AppendMenuW(
                submenu,
                MF_STRING | MF_DISABLED | MF_GRAYED,
                0,
                w!("(Empty)"),
            )?;
        } else {
            for (index, entry) in entries.iter().enumerate() {
                let id = ctx.next_id;
                ctx.next_id += 1;
                let label = HSTRING::from(decorate(&entry.name, ctx.numeric, index + 1));
                AppendMenuW(submenu, MF_STRING, id, PCWSTR(label.as_ptr()))?;
                if let Some(bitmap) = crate::icon::bitmap_for(&entry.path) {
                    set_item_bitmap(submenu, id, bitmap);
                }
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
        let label = HSTRING::from(name);
        AppendMenuW(parent, MF_POPUP, submenu.0 as usize, PCWSTR(label.as_ptr()))?;
        // 先頭項目のアイコンを借りると中身次第で毎回変わり、
        // Recent と Frequent が見分けられない。メニュー種別で固定する
        if let Some(bitmap) = crate::icon::bitmap_for_stock(path_menu_icon(name)) {
            set_last_item_bitmap(parent, bitmap);
        }
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
            AppendMenuW(
                submenu,
                MF_STRING | MF_DISABLED | MF_GRAYED,
                0,
                w!("(Empty)"),
            )?;
        } else {
            for (index, entry) in entries.iter().enumerate() {
                let id = ctx.next_id;
                ctx.next_id += 1;
                let label = HSTRING::from(decorate(&entry.title, ctx.numeric, index + 1));
                AppendMenuW(submenu, MF_STRING, id, PCWSTR(label.as_ptr()))?;
                if let Some(bitmap) = crate::icon::bitmap_for_window(HWND(entry.hwnd as *mut _))
                    .or_else(|| crate::icon::bitmap_for_asset("window", ICON_WINDOW))
                {
                    set_item_bitmap(submenu, id, bitmap);
                }
                ctx.actions
                    .insert(id, Action::ActivateWindow { hwnd: entry.hwnd });
            }
        }
        append_close(submenu)?;
        let label = HSTRING::from(name);
        AppendMenuW(parent, MF_POPUP, submenu.0 as usize, PCWSTR(label.as_ptr()))?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("window", ICON_WINDOW) {
            set_last_item_bitmap(parent, bitmap);
        }
        Ok(())
    }
}

unsafe fn append_close(menu: HMENU) -> Result<()> {
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_CLOSE, w!("Close this menu"))?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("close", ICON_CLOSE) {
            set_item_bitmap(menu, ID_CLOSE, bitmap);
        }
        Ok(())
    }
}

/// QAP と同様に、頻繁に使う管理操作をルートメニューの末尾へ置く。
unsafe fn append_footer(menu: HMENU) -> Result<()> {
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        let settings = HSTRING::from("Settings...");
        AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(settings.as_ptr()))?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("settings", ICON_SETTINGS) {
            set_last_item_bitmap(menu, bitmap);
        }

        let reload = HSTRING::from("Reload config");
        AppendMenuW(menu, MF_STRING, ID_RELOAD, PCWSTR(reload.as_ptr()))?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("reload", ICON_RELOAD) {
            set_item_bitmap(menu, ID_RELOAD, bitmap);
        }

        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        let close = HSTRING::from("Close this menu");
        AppendMenuW(menu, MF_STRING, ID_CLOSE, PCWSTR(close.as_ptr()))?;
        if let Some(bitmap) = crate::icon::bitmap_for_asset("close", ICON_CLOSE) {
            set_last_item_bitmap(menu, bitmap);
        }
        Ok(())
    }
}

/// 項目にアイコンを設定する (FR-2.3) 。
unsafe fn set_item_bitmap(menu: HMENU, id: usize, bmp: HBITMAP) {
    unsafe {
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            hbmpItem: bmp,
            ..Default::default()
        };
        let _ = SetMenuItemInfoW(menu, id as u32, false, &info);
    }
}

unsafe fn set_last_item_bitmap(menu: HMENU, bmp: HBITMAP) {
    unsafe {
        let position = GetMenuItemCount(Some(menu)) - 1;
        if position < 0 {
            return;
        }
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            hbmpItem: bmp,
            ..Default::default()
        };
        let _ = SetMenuItemInfoW(menu, position as u32, true, &info);
    }
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
    use super::{decorate, path_menu_icon, with_branch};

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
}
