//! スタートメニューのショートカット (`.lnk`) を列挙する (FR-9.14)。
//!
//! `docs/spec.md` の非目標どおり、ファイルシステムの全件走査はしない。
//! ユーザー用・全ユーザー用の 2 つのスタートメニューフォルダだけを
//! 再帰的に見る (Windows のスタートメニュー自体がこの 2 箇所の合成)。
//!
//! `scan()` は `.lnk` の実体解決に COM (`IShellLinkW`) を使うため、
//! 呼び出し前に STA COM が初期化されている必要がある
//! (`main.rs` の `shell::ComGuard`) 。未初期化のまま呼ぶと
//! `CoCreateInstance` が静かに失敗し、全件が壊れたリンク扱いで
//! 除外される (実測で確認済み)。

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    ShellLink,
};
use windows::core::{Interface, PCWSTR};

/// 起動可能な 1 アプリ (スタートメニューのショートカット 1 件)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// ショートカットのファイル名 (拡張子抜き)。
    pub name: String,
    /// ショートカット自体のパス。起動はこれを `ShellExecuteW` に渡す
    /// (リンク先の exe を直接叩くと、アプリが期待する作業ディレクトリや
    /// 引数を失うことがあるため)。
    pub shortcut_path: String,
}

/// 2 つのスタートメニューを合わせて列挙する。
/// アンインストール後も残った壊れたショートカットは除く。
pub fn scan() -> Vec<App> {
    let mut apps = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in start_menu_roots() {
        collect(&root, &mut apps, &mut seen);
    }
    apps.retain(|app| target_exists(&app.shortcut_path));
    apps
}

fn start_menu_roots() -> Vec<PathBuf> {
    [
        known_folder_path(&FOLDERID_Programs),
        known_folder_path(&FOLDERID_CommonPrograms),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn known_folder_path(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        // to_string() が失敗しても COM が確保したバッファは解放する。
        // `?` で早期リターンすると CoTaskMemFree に到達せずリークする
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0.cast()));
        path.map(PathBuf::from)
    }
}

pub fn collect(dir: &Path, out: &mut Vec<App>, seen: &mut std::collections::HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, seen);
            continue;
        }
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        // 同名ショートカットはユーザー版を優先 (先に列挙される Programs 側が勝つ)
        let key = name.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(App {
            name,
            shortcut_path: path.to_string_lossy().into_owned(),
        });
    }
}

/// ショートカットの実体を持っているか (壊れたリンクの除外に使う)。
///
/// スタートメニューにはアンインストール後も残った `.lnk` が紛れることが
/// あるため、選択時ではなく列挙時に軽く弾く。`IShellLinkW::GetPath` は
/// ファイル I/O を伴うため、この確認自体は起動時 1 回だけ行う
/// (`dynamic::refresh` と同じ非表示経路)。
fn target_exists(shortcut_path: &str) -> bool {
    resolve_target(shortcut_path).is_some_and(|target| Path::new(&target).exists())
}

fn resolve_target(shortcut_path: &str) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist = link.cast::<IPersistFile>().ok()?;
        let wide: Vec<u16> = Path::new(shortcut_path)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut target = vec![0u16; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
        let len = target.iter().position(|ch| *ch == 0)?;
        (len > 0).then(|| String::from_utf16_lossy(&target[..len]))
    }
}
