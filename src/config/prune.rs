//! 存在しなくなった `folder` / `file` 項目の削除 (FR-7.8)。
//!
//! 存在確認はファイル I/O (ネットワークパスでは数秒固まりうる) なので、
//! 呼び出し側は `item_paths` で集めたパスを `find_missing` へ別スレッドで
//! 渡し、結果だけを UI スレッドで `remove_paths` に適用する。

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Config, Item, expand};

impl Config {
    /// 存在確認の対象になる展開済みパス (メニュー階層全体)。
    /// 変数を解決できない項目は判定しようがないので含めない。
    pub fn item_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        collect_paths(&self.items, &self.variables, &mut paths);
        paths
    }

    /// 展開後のパスが `missing` のいずれかと一致する (大文字小文字を
    /// 区別しない) 項目を階層全体から取り除き、取り除いた項目の
    /// (名前, パス) を返す。
    pub fn remove_paths(&mut self, missing: &[String]) -> Vec<(String, String)> {
        let mut removed = Vec::new();
        let missing: HashSet<String> = missing.iter().map(|path| path.to_lowercase()).collect();
        remove_in(&mut self.items, &self.variables, &missing, &mut removed);
        removed
    }
}

fn collect_paths(items: &[Item], vars: &BTreeMap<String, String>, out: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Folder { path, .. } | Item::File { path, .. } => {
                if let Some(path) = expand(path, vars) {
                    out.push(path);
                }
            }
            Item::Submenu { items, .. } => collect_paths(items, vars, out),
            _ => {}
        }
    }
}

fn remove_in(
    items: &mut Vec<Item>,
    vars: &BTreeMap<String, String>,
    missing: &HashSet<String>,
    removed: &mut Vec<(String, String)>,
) {
    items.retain(|item| match item {
        Item::Folder { name, path, .. } | Item::File { name, path, .. } => {
            let Some(expanded) = expand(path, vars) else {
                return true;
            };
            if missing.contains(&expanded.to_lowercase()) {
                removed.push((name.clone(), expanded));
                false
            } else {
                true
            }
        }
        _ => true,
    });
    for item in items {
        if let Item::Submenu { items, .. } = item {
            remove_in(items, vars, missing, removed);
        }
    }
}

/// 存在確認を並列に走らせるスレッド数。到達できないネットワークパスは 1 件
/// で数秒固まるため、直列だとその間ほかの確認が全部待たされる。
const EXISTS_THREADS: usize = 4;

/// `paths` のうち「ルートは存在するのに本体が無い」ものを返す。
/// ルートが見えない (ドライブ未接続・VPN 切断) 場合と相対パスは
/// 一時的な不在と区別できないので判定しない。
///
/// ルートはパスごとではなく種類ごとに 1 回だけ確認する (到達できない共有に
/// 属する項目が複数あっても、固まるのは 1 回で済む)。確認は並列で行う。
pub fn find_missing(paths: &[String], exists: impl Fn(&Path) -> bool + Sync) -> Vec<String> {
    let roots: Vec<Option<PathBuf>> = paths
        .iter()
        .map(|path| root_of(Path::new(path.as_str())))
        .collect();
    let mut unique: Vec<&PathBuf> = roots.iter().flatten().collect();
    unique.sort();
    unique.dedup();
    let reachable: HashSet<&PathBuf> = unique
        .iter()
        .copied()
        .zip(par_map(&unique, |root| exists(root)))
        .filter_map(|(root, alive)| alive.then_some(root))
        .collect();

    let candidates: Vec<usize> = (0..paths.len())
        .filter(|&index| {
            roots[index]
                .as_ref()
                .is_some_and(|root| reachable.contains(root))
        })
        .collect();
    let gone = par_map(&candidates, |&index| {
        !exists(Path::new(paths[index].as_str()))
    });
    candidates
        .into_iter()
        .zip(gone)
        .filter(|(_, gone)| *gone)
        .map(|(index, _)| paths[index].clone())
        .collect()
}

/// `items` の各要素に `f` を並列に適用し、元の順序で結果を返す。
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..EXISTS_THREADS.min(items.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    let result = f(item);
                    results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((index, result));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// `C:\foo` → `C:\`、`\\server\share\foo` → `\\server\share\`。
fn root_of(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let prefix = match components.next()? {
        Component::Prefix(prefix) => prefix,
        _ => return None,
    };
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    let mut root = PathBuf::from(prefix.as_os_str());
    root.push(Component::RootDir);
    Some(root)
}
