//! 存在しなくなった `folder` / `file` 項目の削除 (FR-7.8)。
//!
//! 存在確認はファイル I/O (ネットワークパスでは数秒固まりうる) なので、
//! 呼び出し側は `item_paths` で集めたパスを `find_missing` へ別スレッドで
//! 渡し、結果だけを UI スレッドで `remove_paths` に適用する。

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

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
        remove_in(&mut self.items, &self.variables, missing, &mut removed);
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
    missing: &[String],
    removed: &mut Vec<(String, String)>,
) {
    items.retain(|item| match item {
        Item::Folder { name, path, .. } | Item::File { name, path, .. } => {
            let Some(expanded) = expand(path, vars) else {
                return true;
            };
            if missing.iter().any(|m| m.eq_ignore_ascii_case(&expanded)) {
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

/// `paths` のうち「ルートは存在するのに本体が無い」ものを返す。
/// ルートが見えない (ドライブ未接続・VPN 切断) 場合と相対パスは
/// 一時的な不在と区別できないので判定しない。
pub fn find_missing(paths: &[String], exists: impl Fn(&Path) -> bool) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            let path = Path::new(path.as_str());
            root_of(path).is_some_and(|root| exists(&root) && !exists(path))
        })
        .cloned()
        .collect()
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
