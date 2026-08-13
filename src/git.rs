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
fn resolve_gitdir_file(text: &str, base: &Path) -> Option<PathBuf> {
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
fn parse_head(text: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_ref() {
        assert_eq!(
            parse_head("ref: refs/heads/main\n").as_deref(),
            Some("main")
        );
    }

    #[test]
    fn keeps_slashes_in_branch_name() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/git-branch\n").as_deref(),
            Some("feature/git-branch")
        );
    }

    #[test]
    fn shortens_detached_head_sha() {
        assert_eq!(
            parse_head("9f8e7d6c5b4a39281706zz").as_deref(),
            None,
            "SHA 以外の文字が混ざるものは表示しない"
        );
        assert_eq!(
            parse_head("9f8e7d6c5b4a3928170695e4d3c2b1a098765432\n").as_deref(),
            Some("9f8e7d6")
        );
    }

    #[test]
    fn ignores_non_branch_refs() {
        assert_eq!(parse_head("ref: refs/tags/v1.0\n"), None);
        assert_eq!(parse_head(""), None);
        assert_eq!(parse_head("ref: refs/heads/\n"), None);
    }

    #[test]
    fn resolves_relative_gitdir_file() {
        let resolved = resolve_gitdir_file(
            "gitdir: ../.git/worktrees/feature\n",
            Path::new("C:\\work\\repo-feature"),
        );
        assert_eq!(
            resolved,
            Some(PathBuf::from(
                "C:\\work\\repo-feature/../.git/worktrees/feature"
            ))
        );
    }

    #[test]
    fn resolves_absolute_gitdir_file() {
        let resolved = resolve_gitdir_file(
            "gitdir: C:\\work\\repo\\.git\\worktrees\\feature\n",
            Path::new("C:\\ignored"),
        );
        assert_eq!(
            resolved,
            Some(PathBuf::from("C:\\work\\repo\\.git\\worktrees\\feature"))
        );
    }

    #[test]
    fn rejects_gitdir_file_without_target() {
        assert_eq!(resolve_gitdir_file("gitdir:\n", Path::new("C:\\x")), None);
        assert_eq!(
            resolve_gitdir_file("something else\n", Path::new("C:\\x")),
            None
        );
    }
}
