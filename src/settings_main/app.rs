//! `SettingsApp` の状態定義とデータ操作 (選択・CRUD・保存)。

use eframe::egui;
use waypoint::config::{Config, Item, LoadOutcome};

use super::drafts::{BatchDraft, ItemDraft};
use super::helpers::{items_at, items_at_mut};
use super::trigger_draft::TriggerDraft;
use super::{AzureProjectPicker, ImportDraft, MoveToMenuDraft, VariablesDraft};

pub(super) struct SettingsApp {
    pub(super) config: Config,
    pub(super) selected_menu: Vec<usize>,
    /// 選択中の行の添字集合。複数選択に対応する (FR-6)。
    pub(super) selected_items: std::collections::BTreeSet<usize>,
    /// 直近でフォーカス/クリックした行。Shift 範囲選択の基準にし、
    /// 単一項目の操作 (Edit 詳細編集など) の対象にも使う。
    pub(super) active_item: Option<usize>,
    /// Shift 範囲選択の起点。Ctrl/Shift を伴わない選択で更新する。
    pub(super) selection_anchor: Option<usize>,
    pub(super) draft: Option<ItemDraft>,
    pub(super) batch_draft: Option<BatchDraft>,
    pub(super) move_to_menu_draft: Option<MoveToMenuDraft>,
    pub(super) variables_draft: Option<VariablesDraft>,
    pub(super) trigger_draft: Option<TriggerDraft>,
    pub(super) azure_project_picker: Option<AzureProjectPicker>,
    pub(super) import_draft: Option<ImportDraft>,
    /// アプリ内クリップボード。Ctrl+C でコピーした項目を保持する。
    pub(super) clipboard: Vec<Item>,
    pub(super) delete_pending: bool,
    pub(super) close_pending: bool,
    pub(super) dirty: bool,
    pub(super) load_error: Option<String>,
    pub(super) status: Option<String>,
    /// この描画で一覧の行がキーボードフォーカスを持っているか。
    /// `Enter` / `Delete` を一覧の操作として扱ってよいかの判定に使う。
    pub(super) focused_row: Option<usize>,
    /// 前の描画での `focused_row`。フォーカスが移った瞬間を見るために持つ。
    pub(super) previous_focused_row: Option<usize>,
    /// 次の描画で選択行へフォーカスを移すか。
    /// 起動直後と、キーボードで選択を動かした直後に立てる。
    pub(super) focus_selected_row: bool,
    /// クリックで選択を確定した直後、フォーカス変化検出による
    /// 単一選択への上書きを 1 回だけ抑止する (Ctrl/Shift クリック用) 。
    pub(super) suppress_focus_select: bool,
}

impl SettingsApp {
    pub(super) fn load() -> Self {
        let (config, load_error) = match waypoint::config::load() {
            LoadOutcome::Loaded(config) | LoadOutcome::Created(config) => (config, None),
            LoadOutcome::Failed(error) => (Config::default(), Some(error)),
        };
        let active_item = (!config.items.is_empty()).then_some(0);
        let selected_items = active_item.into_iter().collect();
        // Quick Launch の `az optimize` から起動された場合、Azure DevOps
        // 設定画面を開いた状態で直接アクティビティ集計を始める
        // (`waypoint::azure_devops::AZURE_SUGGEST_ARG` は常駐部と共有する定数)。
        let mut azure_project_picker = std::env::args()
            .any(|arg| arg == waypoint::azure_devops::AZURE_SUGGEST_ARG)
            .then(|| AzureProjectPicker::from_config(&config));
        if let Some(picker) = azure_project_picker.as_mut() {
            picker.start_priority_suggestions();
        }
        Self {
            config,
            selected_menu: Vec::new(),
            selected_items,
            active_item,
            selection_anchor: active_item,
            draft: None,
            batch_draft: None,
            move_to_menu_draft: None,
            variables_draft: None,
            trigger_draft: None,
            azure_project_picker,
            import_draft: None,
            clipboard: Vec::new(),
            delete_pending: false,
            close_pending: false,
            dirty: false,
            load_error,
            status: None,
            focused_row: None,
            previous_focused_row: None,
            // 起動直後から矢印キーで一覧をたどれるように、選択行へフォーカスを置く
            focus_selected_row: true,
            suppress_focus_select: false,
        }
    }

    /// 単一項目を選択する。クリック (修飾キーなし) やキーボード移動で使う。
    pub(super) fn select_single(&mut self, index: usize) {
        self.selected_items.clear();
        self.selected_items.insert(index);
        self.active_item = Some(index);
        self.selection_anchor = Some(index);
    }

    /// Ctrl+クリック相当。1 件だけ選択集合へ追加/除外する。
    pub(super) fn toggle_selection(&mut self, index: usize) {
        if !self.selected_items.remove(&index) {
            self.selected_items.insert(index);
        }
        self.active_item = Some(index);
        self.selection_anchor = Some(index);
    }

    /// Shift+クリック / Shift+矢印相当。アンカーから index までを選択する。
    pub(super) fn extend_selection_to(&mut self, index: usize) {
        let anchor = self.selection_anchor.unwrap_or(index);
        let (from, to) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };
        self.selected_items.clear();
        self.selected_items.extend(from..=to);
        self.active_item = Some(index);
        // アンカーは動かさない。連続 Shift+矢印で範囲が伸び縮みするようにする
    }

    pub(super) fn select_all(&mut self) {
        let Some(len) = self.current_items().map(Vec::len) else {
            return;
        };
        self.selected_items = (0..len).collect();
        if self.active_item.is_none() {
            self.active_item = len.checked_sub(1);
        }
    }

    /// 選択集合を添字の昇順で返す。
    pub(super) fn selected_indices(&self) -> Vec<usize> {
        self.selected_items.iter().copied().collect()
    }

    pub(super) fn selection_summary(&self) -> String {
        let count = self.selected_items.len();
        if count <= 1 {
            "The selected item".to_string()
        } else {
            format!("The {count} selected items")
        }
    }

    pub(super) fn current_items(&self) -> Option<&Vec<Item>> {
        items_at(&self.config, &self.selected_menu)
    }

    pub(super) fn current_items_mut(&mut self) -> Option<&mut Vec<Item>> {
        items_at_mut(&mut self.config, &self.selected_menu)
    }

    pub(super) fn save(&mut self) -> bool {
        if self.load_error.is_some() {
            return false;
        }
        match waypoint::config::save(&self.config) {
            Ok(()) => {
                self.dirty = false;
                self.status = Some(if waypoint::tray::signal_reload() {
                    "Saved and reloaded.".to_string()
                } else {
                    "Saved. Waypoint is not running.".to_string()
                });
                true
            }
            Err(error) => {
                self.status = Some(format!("Save failed: {error}"));
                false
            }
        }
    }

    pub(super) fn open_variables(&mut self) {
        self.variables_draft = Some(VariablesDraft {
            entries: self
                .config
                .variables
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            error: None,
        });
    }

    pub(super) fn open_trigger(&mut self) {
        self.trigger_draft = Some(TriggerDraft::from_config(&self.config));
    }

    pub(super) fn open_azure_settings(&mut self) {
        self.azure_project_picker = Some(AzureProjectPicker::from_config(&self.config));
    }

    pub(super) fn open_import(&mut self) {
        self.import_draft = Some(ImportDraft {
            root: String::new(),
            depth: 2,
            preview: None,
            error: None,
        });
    }

    pub(super) fn request_close(&mut self, ctx: &egui::Context) {
        if self.dirty {
            self.close_pending = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// いずれかのダイアログ (モーダル相当) を表示中か。
    pub(super) fn any_dialog_open(&self) -> bool {
        self.draft.is_some()
            || self.batch_draft.is_some()
            || self.move_to_menu_draft.is_some()
            || self.variables_draft.is_some()
            || self.trigger_draft.is_some()
            || self.azure_project_picker.is_some()
            || self.import_draft.is_some()
            || self.delete_pending
            || self.close_pending
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_pending = true;
        }
        self.add_dropped_folders(&ctx);
        // Shift+矢印は egui の自動フォーカス移動より先に横取りする必要がある
        self.handle_range_selection_keys(&ctx);
        // それ以外のショートカットは一覧を描いた後に見る。先に打鍵を消費すると、
        // フォーカス中のウィジェットが Enter で起動できなくなる
        self.show_items(ui);
        self.handle_shortcuts(&ctx);
        self.show_item_editor(&ctx);
        self.show_batch_editor(&ctx);
        self.show_move_to_menu_editor(&ctx);
        self.show_variables_editor(&ctx);
        self.show_trigger_editor(&ctx);
        self.show_azure_project_picker(&ctx);
        self.show_import_editor(&ctx);
        self.show_confirmations(&ctx);
    }
}
