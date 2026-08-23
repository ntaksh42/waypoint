//! メニュー項目の型定義。

use serde::{Deserialize, Serialize};

/// 項目の型。JSON の "type" で判別する。
///
/// `rename_all` は列挙子の名前 (folder / specialFolder …) に効く。
/// フィールド名は別途 `rename_all_fields` で camelCase にする
/// (known_folder → knownFolder)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Item {
    /// 通常のフォルダ。パスは変数を含んでよい。
    Folder {
        name: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        /// Git ブランチ名を表示するか (FR-2.14) 。
        /// 項目ごとのオプトイン。既定の偽では `.git` を一切読まない。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show_branch: bool,
    },
    /// 通常のファイル。パスは変数を含んでよい。
    /// `newWindow`/`reuse` の概念を持たず、既定の関連付けアプリで開く。
    File {
        name: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    /// 既知フォルダ (Downloads, Desktop など) 。
    SpecialFolder {
        name: String,
        known_folder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenMode>,
    },
    /// shell 名前空間への入口 (This PC, Network, Control Panel, Recycle Bin など) 。
    /// `target` は `ShellExecuteW` にそのまま渡す (`shell:MyComputerFolder` 等) 。
    Shell { name: String, target: String },
    /// 入れ子のメニュー。
    Submenu {
        name: String,
        #[serde(default)]
        items: Vec<Item>,
        /// 配下の Folder 項目すべてに showBranch を継承させる (FR-2.14) 。
        /// 個々の Folder 側の showBranch との OR。既定は偽
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show_branch: bool,
    },
    /// 区切り線。name があれば見出しとして描く。
    Separator {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl Item {
    /// メニューに表示する文字列。separator の name なしは None。
    pub fn label(&self) -> Option<&str> {
        match self {
            Item::Folder { name, .. }
            | Item::File { name, .. }
            | Item::SpecialFolder { name, .. }
            | Item::Shell { name, .. }
            | Item::Submenu { name, .. } => Some(name),
            Item::Separator { name } => name.as_deref(),
        }
    }
}

/// フォルダの開き方。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum OpenMode {
    /// 新しいエクスプローラーウィンドウで開く。
    #[default]
    NewWindow,
    /// 元ウィンドウのフォルダを変更する。
    Reuse,
}
