//! Win32 の `HMENU` を構築する処理。

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::Shell::SIID_FOLDER;
use windows::Win32::UI::Shell::SIID_STACK;
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, GetMenuItemCount, HMENU, InsertMenuItemW, MENU_ITEM_FLAGS, MENUITEMINFOW,
    MF_DISABLED, MF_GRAYED, MF_POPUP, MF_STRING, MFS_DISABLED, MFT_OWNERDRAW, MFT_SEPARATOR,
    MIIM_DATA, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_SUBMENU,
};
use windows::core::Result;

use crate::config::{Item, OpenMode};
use crate::dynamic::{Menus as DynamicMenus, PathEntry, WindowEntry};

use super::label::{decorate, path_menu_icon};
use super::{
    Action, BuildCtx, ICON_CLOSE, ICON_RELOAD, ICON_WINDOW, ID_CLOSE, ID_RELOAD, ID_SETTINGS,
};

/// `inherited_show_branch` は親 Submenu (祖先を含む) の showBranch が
/// 真だったかどうか。真なら配下の Folder は自身の showBranch を問わず
/// ブランチ名を表示する (FR-2.14) 。
pub(crate) unsafe fn build_level(
    items: &[Item],
    inherited_show_branch: bool,
    ctx: &mut BuildCtx,
) -> Result<HMENU> {
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
                Item::Submenu {
                    name,
                    items,
                    show_branch,
                } => {
                    let sub = build_level(items, inherited_show_branch || *show_branch, ctx)?;
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
                        .filter(|_| inherited_show_branch || *show_branch)
                        .and_then(crate::git::branch_of);
                    let label = crate::git::with_branch(name, branch.as_deref());
                    append_leaf(menu, ctx, &label, resolved, open.unwrap_or_default(), accel)?;
                }
                Item::File { name, path, .. } => {
                    accel += 1;
                    let resolved = crate::config::expand(path, ctx.vars);
                    append_leaf(menu, ctx, name, resolved, OpenMode::NewWindow, accel)?;
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
pub(crate) unsafe fn append_leaf(
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
pub(crate) unsafe fn append_in_the_works(
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

pub(crate) unsafe fn append_path_menu(
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

pub(crate) unsafe fn append_window_menu(
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

pub(crate) unsafe fn append_close(menu: HMENU) -> Result<()> {
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
pub(crate) unsafe fn append_footer(menu: HMENU) -> Result<()> {
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
pub(crate) unsafe fn append_owner_drawn(
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
            text: super::label::strip_accelerator(label),
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
pub(crate) unsafe fn append_separator(menu: HMENU) -> Result<()> {
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
