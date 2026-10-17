//! 入力されたパスの候補化と、Tab による階層ナビゲーション用の純関数 (FR-9.23〜FR-9.25)。
//!
//! Win32 にもファイルシステムにも触れない。存在確認は FR-9.4 の制約に従い
//! 入力中には行わず、列挙は呼び出し側が Tab 操作の明示時にだけ別スレッドで行う。

use super::{Action, Entry};
use crate::config::OpenMode;

/// 入力を開けるパスとして正規化する。パスらしくなければ `None`。
///
/// 前後の空白と二重引用符 (エクスプローラーの「パスのコピー」が付ける) を除き、
/// `/` を `\` に揃える。`X:\` で始まるか、`\` で始まる (UNC) ものだけを通す。
/// 相対パス・`C:` 単独・URL は通さない (誤爆防止)。
pub fn normalize_path_input(query: &str) -> Option<String> {
    let trimmed = query.trim().trim_matches('"').trim();
    let path = trimmed.replace('/', "\\");
    let mut chars = path.chars();
    let first = chars.next()?;
    let is_drive =
        first.is_ascii_alphabetic() && chars.next() == Some(':') && chars.next() == Some('\\');
    let is_unc = first == '\\' && path.len() > 1;
    (is_drive || is_unc).then_some(path)
}

/// 入力が開けるパスなら確定候補 1 件を作る (FR-9.23)。
pub fn path_entry(query: &str) -> Option<Entry> {
    let path = normalize_path_input(query)?;
    let last = path
        .trim_end_matches('\\')
        .rsplit('\\')
        .next()
        .unwrap_or("");
    // ルート (`C:\`) と UNC の先頭 (`\\server`) はセグメント名が名前にならない
    let name = if last.is_empty() || last.ends_with(':') {
        path.clone()
    } else {
        last.to_string()
    };
    Some(Entry {
        azure: None,
        name,
        breadcrumb: "Path".to_string(),
        path,
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    })
}

/// 1 階層上のパス (末尾 `\` 付き)。これ以上上がれない (ルート) なら `None`。
///
/// UNC は `\\server\share\` をルートとして扱い、そこより上へは上がらない。
pub fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('\\');
    let cut = trimmed.rfind('\\')?;
    let parent = &trimmed[..=cut];
    // UNC は共有名より手前へは上がらない (`server\` のように区切りが 2 つ未満)
    let is_above_share = parent
        .strip_prefix("\\\\")
        .is_some_and(|unc| unc.matches('\\').count() < 2);
    (!is_above_share).then(|| parent.to_string())
}

/// 入力パスを「列挙するディレクトリ (末尾 `\` 付き)」と「最後のセグメントの途中入力」に分ける。
pub fn split_dir_prefix(path: &str) -> (&str, &str) {
    match path.rfind('\\') {
        Some(cut) => path.split_at(cut + 1),
        None => ("", path),
    }
}

/// 2 つのパスが大文字小文字を除いて同じか (Windows のパス比較)。
pub fn same_dir(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// `prefix` に前方一致 (大文字小文字無視) する名前を、元の並びのまま返す。
pub fn dirs_matching<'a>(names: &'a [String], prefix: &str) -> Vec<&'a String> {
    let prefix = prefix.to_lowercase();
    names
        .iter()
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .collect()
}

/// 候補名の共通接頭辞 (大文字小文字無視で比較し、先頭の名前の表記を返す)。
pub fn common_prefix(names: &[&String]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut len = first.chars().count();
    for name in &names[1..] {
        len = first
            .chars()
            .zip(name.chars())
            .take(len)
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
    }
    first.chars().take(len).collect()
}

/// サブディレクトリ 1 件を候補にする。
pub fn dir_entry(dir: &str, name: &str) -> Entry {
    Entry {
        azure: None,
        name: name.to_string(),
        breadcrumb: String::new(),
        path: format!("{dir}{name}"),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }
}
