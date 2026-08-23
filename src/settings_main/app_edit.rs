//! 項目の追加・編集・複製・削除・並べ替え・クリップボード操作。

use eframe::egui;
use waypoint::config::Item;

use super::app::SettingsApp;
use super::drafts::{BatchDraft, DraftKind, ItemDraft};
use super::helpers::items_at_mut;

impl SettingsApp {
    pub(super) fn begin_add(&mut self, kind: DraftKind) {
        self.draft = Some(ItemDraft::new(kind));
    }

    /// 単一選択なら詳細編集、複数選択ならバッチ編集ダイアログを開く。
    pub(super) fn begin_edit(&mut self) {
        if self.selected_items.len() > 1 {
            self.begin_batch_edit();
            return;
        }
        let Some(index) = self.active_item else {
            return;
        };
        if let Some(item) = self.current_items().and_then(|items| items.get(index)) {
            self.draft = Some(ItemDraft::from_item(index, item));
        }
    }

    pub(super) fn begin_batch_edit(&mut self) {
        if self.selected_items.len() < 2 {
            return;
        }
        self.batch_draft = Some(BatchDraft::default());
    }

    pub(super) fn apply_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let editing = draft.editing;
        let item = draft.into_item();
        let Some(items) = self.current_items_mut() else {
            return;
        };

        let selected = if let Some(index) = editing {
            if let Some(slot) = items.get_mut(index) {
                *slot = item;
                Some(index)
            } else {
                None
            }
        } else {
            items.push(item);
            Some(items.len() - 1)
        };
        if let Some(index) = selected {
            self.select_single(index);
        }
        self.dirty = true;
        self.status = None;
    }

    /// バッチ編集ダイアログの内容を選択中の全項目へ適用する (FR-6) 。
    pub(super) fn apply_batch_draft(&mut self) {
        let Some(draft) = self.batch_draft.take() else {
            return;
        };
        let indices: std::collections::BTreeSet<usize> = self.selected_items.clone();
        let Some(items) = self.current_items_mut() else {
            return;
        };
        for &index in &indices {
            let Some(item) = items.get_mut(index) else {
                continue;
            };
            if let Some(mode) = draft.open {
                match item {
                    Item::Folder { open, .. } | Item::SpecialFolder { open, .. } => {
                        *open = Some(mode);
                    }
                    _ => {}
                }
            }
            if let Some(show_branch) = draft.show_branch {
                match item {
                    Item::Folder {
                        show_branch: field, ..
                    }
                    | Item::Submenu {
                        show_branch: field, ..
                    } => {
                        *field = show_branch;
                    }
                    _ => {}
                }
            }
        }
        self.dirty = true;
        self.status = None;
    }

    /// QAP 同等構成の「My Special Folders」をワンショットで挿入する (FR-6) 。
    pub(super) fn insert_my_special_folders_preset(&mut self) {
        if let Some(items) = self.current_items_mut() {
            items.push(waypoint::config::my_special_folders_item());
            let index = items.len() - 1;
            self.select_single(index);
            self.dirty = true;
            self.status = None;
        }
    }

    /// 選択中の全項目を直後にまとめて複製する。
    pub(super) fn duplicate_selected(&mut self) {
        if self.selected_items.is_empty() {
            return;
        }
        let indices = self.selected_indices();
        let Some(items) = self.current_items_mut() else {
            return;
        };
        // 後ろから挿入すると前側の添字がずれない
        let mut new_selection = std::collections::BTreeSet::new();
        for &index in indices.iter().rev() {
            let Some(item) = items.get(index).cloned() else {
                continue;
            };
            items.insert(index + 1, item);
        }
        // 複製後の新しい添字を選択する: 各元位置は自身の並びの中で index 個前に
        // 挿入が起きた分だけ前方にずれるので、元の相対順で1つずつ後ろへ計算し直す
        for (offset, &index) in indices.iter().enumerate() {
            new_selection.insert(index + offset + 1);
        }
        self.selected_items = new_selection;
        self.active_item = self.selected_items.iter().next_back().copied();
        self.selection_anchor = self.active_item;
        self.dirty = true;
        self.status = None;
    }

    /// 選択中の全項目をまとめて削除する。
    pub(super) fn remove_selected(&mut self) {
        if self.selected_items.is_empty() {
            return;
        }
        let indices = self.selected_indices();
        let Some(items) = self.current_items_mut() else {
            return;
        };
        for &index in indices.iter().rev() {
            if index < items.len() {
                items.remove(index);
            }
        }
        let first_removed = indices.first().copied().unwrap_or(0);
        let next = if first_removed < items.len() {
            Some(first_removed)
        } else {
            first_removed.checked_sub(1).filter(|_| !items.is_empty())
        };
        self.selected_items.clear();
        if let Some(next) = next {
            self.selected_items.insert(next);
        }
        self.active_item = next;
        self.selection_anchor = next;
        self.dirty = true;
        self.status = None;
        // 削除後の再描画では egui 側のフォーカスがまだ古い index の行を指しており、
        // それを「フォーカス変化」と誤検出した show_item_rows が select_single で
        // ここで決めた選択を上書きしてしまう。クリック時と同じガードで防ぐ
        self.suppress_focus_select = true;
    }

    /// 選択中の全項目をまとめて上下へ動かす。連続していなくてもよい。
    ///
    /// 端 (先頭/末尾) に達した項目が一つでもあれば全体を動かさない。
    /// バラバラな選択でも相対順序を保ったまま 1 段ずつ動かす。
    pub(super) fn move_selected(&mut self, delta: isize) {
        if self.selected_items.is_empty() || delta == 0 {
            return;
        }
        let indices = self.selected_indices();
        let Some(items) = self.current_items_mut() else {
            return;
        };
        let len = items.len();
        // 端で止まるかの判定
        if delta < 0 {
            if indices.first().is_some_and(|&first| first == 0) {
                return;
            }
        } else if indices.last().is_some_and(|&last| last + 1 >= len) {
            return;
        }

        let mut new_selection = std::collections::BTreeSet::new();
        if delta < 0 {
            for &index in &indices {
                items.swap(index, index - 1);
                new_selection.insert(index - 1);
            }
        } else {
            for &index in indices.iter().rev() {
                items.swap(index, index + 1);
                new_selection.insert(index + 1);
            }
        }
        self.selected_items = new_selection;
        self.active_item = self
            .active_item
            .map(|active| active.checked_add_signed(delta).unwrap_or(active));
        self.selection_anchor = self.active_item;
        self.dirty = true;
        self.status = None;
    }

    /// ドラッグした行を挿入位置へ移す (FR-6.3) 。
    ///
    /// `insert_at` は「元の並びで何番目の手前に入れるか」なので、
    /// 取り除く操作で後ろの添字が 1 つ詰まる分を補正する。
    pub(super) fn reorder(&mut self, from: usize, insert_at: usize) {
        let Some(items) = self.current_items_mut() else {
            return;
        };
        if from >= items.len() || insert_at > items.len() {
            return;
        }
        let target = super::reorder_target(from, insert_at);
        if target == from {
            return;
        }
        let item = items.remove(from);
        items.insert(target, item);
        self.select_single(target);
        self.dirty = true;
        self.status = None;
    }

    pub(super) fn add_dropped_folders(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let item = if path.is_dir() {
                Item::Folder {
                    name,
                    path: path.display().to_string(),
                    open: None,
                    icon: None,
                    show_branch: false,
                }
            } else if path.is_file() {
                Item::File {
                    name,
                    path: path.display().to_string(),
                    icon: None,
                }
            } else {
                continue;
            };
            if let Some(items) = self.current_items_mut() {
                items.push(item);
                let index = items.len() - 1;
                self.select_single(index);
                self.dirty = true;
                self.status = None;
            }
        }
    }

    /// 選択中の全項目をアプリ内クリップボードへコピーする。
    pub(super) fn copy_selected(&mut self) {
        let Some(items) = self.current_items() else {
            return;
        };
        let copied: Vec<Item> = self
            .selected_indices()
            .into_iter()
            .filter_map(|index| items.get(index).cloned())
            .collect();
        if !copied.is_empty() {
            self.clipboard = copied;
        }
    }

    /// クリップボードの内容を現在のメニューの末尾へ貼り付ける。
    pub(super) fn paste_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let pasted = self.clipboard.clone();
        let Some(items) = self.current_items_mut() else {
            return;
        };
        let start = items.len();
        items.extend(pasted);
        self.selected_items = (start..items.len()).collect();
        self.active_item = self.selected_items.iter().next_back().copied();
        self.selection_anchor = self.active_item;
        self.dirty = true;
        self.status = None;
    }

    /// 選択中の全項目を、選んだ別メニュー (サブメニュー) の末尾へ移動する (FR-6.3) 。
    ///
    /// `target` が移動対象のいずれかの配下 (自分自身を含む) を指す場合は
    /// 移動後に経路が失われるため何もしない。
    pub(super) fn move_selected_to_menu(&mut self, target: &[usize]) {
        if self.selected_items.is_empty() || target == self.selected_menu.as_slice() {
            return;
        }
        let indices = self.selected_indices();
        if target.len() > self.selected_menu.len()
            && target[..self.selected_menu.len()] == *self.selected_menu
            && indices.contains(&target[self.selected_menu.len()])
        {
            return;
        }
        // target が移動元と同じ階層 (兄弟) を指す場合、削除で後ろの添字が
        // 詰まる分だけ補正しないとターゲットを取り違える
        let mut target = target.to_vec();
        if target.len() > self.selected_menu.len()
            && target[..self.selected_menu.len()] == *self.selected_menu
        {
            let sibling = target[self.selected_menu.len()];
            let removed_before = indices.iter().filter(|&&i| i < sibling).count();
            target[self.selected_menu.len()] -= removed_before;
        }

        let Some(source) = items_at_mut(&mut self.config, &self.selected_menu) else {
            return;
        };
        let mut moving = Vec::with_capacity(indices.len());
        for &index in indices.iter().rev() {
            if index < source.len() {
                moving.insert(0, source.remove(index));
            }
        }
        if moving.is_empty() {
            return;
        }
        let Some(destination) = items_at_mut(&mut self.config, &target) else {
            return;
        };
        destination.extend(moving);
        self.selected_items.clear();
        self.active_item = None;
        self.selection_anchor = None;
        self.dirty = true;
        self.status = None;
    }
}
