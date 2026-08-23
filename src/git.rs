//! Git のブランチ名を取得する (FR-2.14 / FR-2.15) 。
//!
//! `git` コマンドは起動しない。プロセス生成は数十 ms かかり、
//! メニュー構築のたびに項目数だけ積み上がるため、表示遅延の目標
//! (50ms) を単独で使い切りうる。`.git/HEAD` を直接読めば済む。
//!
//! 読むのは `showBranch` を有効にした項目だけ。全項目を走査すると
//! ネットワークパスの項目でメニュー構築が固まる。

use std::path::{Path, PathBuf};

/// detached HEAD で表示する SHA の桁数。git の既定の短縮形に合わせる。
const SHORT_SHA_LEN: usize = 7;

/// パスを含む Git 作業ツリーのブランチ名を返す。
///
/// リポジトリでなければ `None`。detached HEAD では短縮 SHA を返す。
pub fn branch_of(path: &str) -> Option<String> {
    let git_dir = find_git_dir(Path::new(path))?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    parse_head(&head)
}

/// ブランチ名があれば項目名の後ろに `[名前]` を付す (FR-2.14) 。
/// リポジトリでない項目は名前のみ。
pub fn with_branch(name: &str, branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("{name}  [{branch}]"),
        None => name.to_string(),
    }
}

/// `.git` を持つ祖先を探す。サブディレクトリを登録していても効くようにする。
///
/// `.git` がファイルの場合はワークツリーかサブモジュールで、中身の
/// `gitdir: <path>` が実体を指す。この場合 HEAD は参照先にある。
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join(".git");
        let meta = std::fs::metadata(&candidate).ok();
        match meta {
            Some(m) if m.is_dir() => return Some(candidate),
            Some(m) if m.is_file() => {
                let text = std::fs::read_to_string(&candidate).ok()?;
                return resolve_gitdir_file(&text, dir);
            }
            _ => {}
        }
    }
    None
}

/// `.git` ファイルの `gitdir:` 行を解決する。
/// 相対パスは `.git` ファイルのあるディレクトリ基準。
pub fn resolve_gitdir_file(text: &str, base: &Path) -> Option<PathBuf> {
    let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    if target.is_empty() {
        return None;
    }
    let path = Path::new(target);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

/// HEAD の中身からブランチ名を取り出す。
///
/// - `ref: refs/heads/<name>` → `<name>` (スラッシュを含む名前もそのまま)
/// - 40 桁の SHA → detached HEAD。短縮して返す
pub fn parse_head(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    if let Some(reference) = line.strip_prefix("ref:") {
        let reference = reference.trim();
        // refs/heads/ 以外 (refs/tags/ など) はブランチではない
        let name = reference.strip_prefix("refs/heads/")?;
        return (!name.is_empty()).then(|| name.to_string());
    }
    // detached HEAD。SHA 以外の想定外の内容は表示しない
    let is_sha = line.len() >= SHORT_SHA_LEN && line.chars().all(|c| c.is_ascii_hexdigit());
    is_sha.then(|| line[..SHORT_SHA_LEN].to_string())
}
