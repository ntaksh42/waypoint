//! 単一項目編集・変数編集・一括編集・移動先選択・インポートの各ダイアログの状態。

use waypoint::config::{Item, OpenMode};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DraftKind {
    Folder,
    File,
    SpecialFolder,
    Shell,
    Submenu,
    Separator,
}

pub(super) struct ItemDraft {
    pub(super) editing: Option<usize>,
    pub(super) kind: DraftKind,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) known_folder: String,
    pub(super) target: String,
    pub(super) open: OpenMode,
    pub(super) icon: Option<String>,
    pub(super) show_branch: bool,
    pub(super) submenu_items: Vec<Item>,
    pub(super) error: Option<String>,
}

pub(super) struct VariablesDraft {
    pub(super) entries: Vec<(String, String)>,
    pub(super) error: Option<String>,
}

/// 複数項目の共有プロパティをまとめて編集するダイアログ (FR-6) 。
/// `None` は「変更しない」を表す。
#[derive(Default)]
pub(super) struct BatchDraft {
    pub(super) open: Option<OpenMode>,
    pub(super) show_branch: Option<bool>,
}

/// 選択項目をまとめて別メニューへ移す先を選ぶダイアログ (FR-6.3) 。
#[derive(Default)]
pub(super) struct MoveToMenuDraft {
    pub(super) target: Vec<usize>,
}

pub(super) struct ImportDraft {
    pub(super) root: String,
    pub(super) depth: usize,
    pub(super) preview: Option<waypoint::folder_import::FolderNode>,
    pub(super) error: Option<String>,
}

impl ItemDraft {
    pub(super) fn new(kind: DraftKind) -> Self {
        Self {
            editing: None,
            kind,
            name: String::new(),
            path: String::new(),
            known_folder: waypoint::known_folder::NAMES[0].to_string(),
            target: String::new(),
            open: OpenMode::default(),
            icon: None,
            show_branch: false,
            submenu_items: Vec::new(),
            error: None,
        }
    }

    pub(super) fn from_item(index: usize, item: &Item) -> Self {
        let mut draft = match item {
            Item::Folder {
                name,
                path,
                open,
                icon,
                show_branch,
            } => Self {
                kind: DraftKind::Folder,
                name: name.clone(),
                path: path.clone(),
                open: open.unwrap_or_default(),
                icon: icon.clone(),
                show_branch: *show_branch,
                ..Self::new(DraftKind::Folder)
            },
            Item::File { name, path, icon } => Self {
                kind: DraftKind::File,
                name: name.clone(),
                path: path.clone(),
                icon: icon.clone(),
                ..Self::new(DraftKind::File)
            },
            Item::SpecialFolder {
                name,
                known_folder,
                open,
            } => Self {
                kind: DraftKind::SpecialFolder,
                name: name.clone(),
                known_folder: known_folder.clone(),
                open: open.unwrap_or_default(),
                ..Self::new(DraftKind::SpecialFolder)
            },
            Item::Shell { name, target } => Self {
                kind: DraftKind::Shell,
                name: name.clone(),
                target: target.clone(),
                ..Self::new(DraftKind::Shell)
            },
            Item::Submenu {
                name,
                items,
                show_branch,
            } => Self {
                kind: DraftKind::Submenu,
                name: name.clone(),
                submenu_items: items.clone(),
                show_branch: *show_branch,
                ..Self::new(DraftKind::Submenu)
            },
            Item::Separator { name } => Self {
                kind: DraftKind::Separator,
                name: name.clone().unwrap_or_default(),
                ..Self::new(DraftKind::Separator)
            },
        };
        draft.editing = Some(index);
        draft
    }

    pub(super) fn validate(&self) -> Option<String> {
        match self.kind {
            DraftKind::Folder if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            DraftKind::Folder if self.path.trim().is_empty() => {
                Some("Path is required.".to_string())
            }
            DraftKind::File if self.name.trim().is_empty() => Some("Name is required.".to_string()),
            DraftKind::File if self.path.trim().is_empty() => Some("Path is required.".to_string()),
            DraftKind::SpecialFolder | DraftKind::Submenu if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            DraftKind::Shell if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            DraftKind::Shell if self.target.trim().is_empty() => {
                Some("Target is required.".to_string())
            }
            _ => None,
        }
    }

    pub(super) fn into_item(self) -> Item {
        let open = (self.open != OpenMode::default()).then_some(self.open);
        match self.kind {
            DraftKind::Folder => Item::Folder {
                name: self.name,
                path: self.path,
                open,
                icon: self.icon,
                show_branch: self.show_branch,
            },
            DraftKind::File => Item::File {
                name: self.name,
                path: self.path,
                icon: self.icon,
            },
            DraftKind::SpecialFolder => Item::SpecialFolder {
                name: self.name,
                known_folder: self.known_folder,
                open,
            },
            DraftKind::Shell => Item::Shell {
                name: self.name,
                target: self.target,
            },
            DraftKind::Submenu => Item::Submenu {
                name: self.name,
                items: self.submenu_items,
                show_branch: self.show_branch,
            },
            DraftKind::Separator => Item::Separator {
                name: (!self.name.trim().is_empty()).then_some(self.name),
            },
        }
    }
}
