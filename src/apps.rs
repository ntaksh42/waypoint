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

use std::collections::HashMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

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
    retain_existing(apps)
}

/// 解決に使うスレッド数。`.lnk` の `IPersistFile::Load` が 1 件ずつ I/O を
/// 待つため、複数スレッドで重ねると待ちが隠れる。
const RESOLVE_WORKERS: usize = 4;

/// 実体のあるショートカットだけを、元の順序を保って残す。
///
/// `IShellLinkW` は STA 前提なので、各スレッドが自前で COM を初期化する。
fn retain_existing(apps: Vec<App>) -> Vec<App> {
    let chunk = apps.len().div_ceil(RESOLVE_WORKERS).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = apps
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    let _com = crate::shell::ComGuard::new();
                    part.iter()
                        .filter(|app| target_exists(&app.shortcut_path))
                        .cloned()
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    })
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
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() && !file_type.is_symlink() {
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
    resolve_target_cached(shortcut_path).is_some_and(|target| Path::new(&target).exists())
}

/// `.lnk` ごとの更新日時と解決したリンク先。
type TargetCache = Mutex<HashMap<String, (SystemTime, String)>>;

/// 解決結果の記憶。設定の再読み込みのたびに全 `.lnk` を `IPersistFile::Load`
/// し直さないための物 (実測で warm 時も 83 件で約 20ms)。リンク先が消えたかは
/// 呼び出し側が毎回 `exists` で見るので、記憶するのは解決結果だけ。
/// 失敗は記憶しない (COM が未初期化の間の失敗などが居座らないように)。
static TARGETS: LazyLock<TargetCache> = LazyLock::new(Mutex::default);

/// 更新日時が前回と同じなら記憶したリンク先を返し、変わっていれば解決し直す。
fn resolve_target_cached(shortcut_path: &str) -> Option<String> {
    let modified = fs::metadata(shortcut_path)
        .and_then(|m| m.modified())
        .ok()?;
    let cached = TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(shortcut_path)
        .filter(|(at, _)| *at == modified)
        .map(|(_, target)| target.clone());
    if cached.is_some() {
        return cached;
    }
    let target = resolve_target(shortcut_path)?;
    TARGETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(shortcut_path.to_string(), (modified, target.clone()));
    Some(target)
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
