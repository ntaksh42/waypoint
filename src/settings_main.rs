//! waypoint 設定エディター (FR-6)。常駐部とは別プロセスで必要時のみ起動する。

#![windows_subsystem = "windows"]

use eframe::egui;
use waypoint::config::{Config, Item, LoadOutcome, OpenMode};
use waypoint::hotkey_capture;

fn main() -> eframe::Result<()> {
    // 設定画面も GUI サブシステム。panic を握り潰さずログへ残す
    waypoint::panic_log::install();

    let icon = app_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 560.0])
            .with_min_inner_size([640.0, 520.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "Customize - Waypoint",
        options,
        Box::new(move |creation| {
            creation.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(SettingsApp::load()))
        }),
    )
}

/// 項目一覧の列幅。見出しと各行で共有する。
const COL_NAME: f32 = 170.0;
const COL_KIND: f32 = 96.0;
const COL_OPEN: f32 = 82.0;
/// 見出し行のグリップ分の字下げ。
const ROW_INDENT: f32 = 20.0;
/// 一覧末尾に置く受け口の高さ。ここへ落とすと最後尾へ移る。
const TAIL_DROP_HEIGHT: f32 = 24.0;

/// ドラッグ中の行。掴んだ時点の添字だけを運ぶ。
#[derive(Debug, Clone, Copy)]
struct DragRow {
    index: usize,
}

/// `Modifiers` を厳密に見て打鍵を 1 つ取り出す。
///
/// egui の `consume_key` は `matches_logically` で照合するため、指定していない
/// Shift / Alt を無視する。素の `Enter` を待っているつもりが `Shift+Enter` まで
/// 拾ってしまうので、ここでは `matches_exact` で見る。
fn consume_key_exact(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| {
        let mut hit = false;
        input.events.retain(|event| {
            let is_match = matches!(
                event,
                egui::Event::Key {
                    key: event_key,
                    modifiers: event_modifiers,
                    pressed: true,
                    ..
                } if *event_key == key && event_modifiers.matches_exact(modifiers)
            );
            hit |= is_match;
            !is_match
        });
        hit
    })
}

/// ダイアログの背後へフォーカスが漏れないようにする。
///
/// `egui::Window` は既定でモーダルではない。開いたまま `Tab` を押すと、
/// フォーカスは画面全体を対象に探され、背後の一覧の行へ抜けてしまう。
/// 実機で確認済み: 削除確認を開いたまま Tab → Enter で、
/// 確認ボタンを押さずに (背後で選択されていた) 別の項目が削除された。
///
/// `Memory::set_modal_layer` はこのレイヤーより下のフォーカス要求を
/// 一律で断る。次のフレームから効くので、毎フレーム呼び続けること。
fn lock_modal_focus<R>(ctx: &egui::Context, window: &Option<egui::InnerResponse<R>>) {
    if let Some(window) = window {
        ctx.memory_mut(|memory| memory.set_modal_layer(window.response.layer_id));
    }
}

/// フォーカス中のウィジェットが `Enter` を自分の起動に使うか。
///
/// egui はフォーカス中のクリック可能なウィジェットを `Enter` で押す。
/// そこへ画面側の既定動作を重ねると、Cancel にフォーカスがある状態の
/// `Enter` が「取り消し」と「確定」の両方を起こしてしまう。
fn focus_takes_enter(ctx: &egui::Context) -> bool {
    let Some(id) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };
    ctx.read_response(id)
        .is_some_and(|response| response.sense.senses_click())
}

/// ダイアログの既定キー。`Enter` で確定、`Esc` で取り消す。
///
/// **ウィンドウを描いた後に呼ぶこと。** 先に消費すると、フォーカス中のボタンへ
/// `Enter` が届かなくなる (egui はイベントを残したまま `key_pressed` で判定する) 。
///
/// `accept` は複数行の入力欄を持つダイアログで `false` にする。そこでは
/// `Enter` が改行であり、確定に使うと文字が打てなくなる。
fn dialog_keys(ctx: &egui::Context, accept: bool) -> (bool, bool) {
    let ok = accept
        && !focus_takes_enter(ctx)
        && consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Enter);
    let cancel = consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Escape);
    (ok, cancel)
}

/// 挿入位置を示す線を引く。`after` なら行の下端、そうでなければ上端。
fn draw_insert_line(ui: &egui::Ui, rect: egui::Rect, after: bool) {
    let y = if after { rect.bottom() } else { rect.top() };
    let stroke = egui::Stroke::new(2.0, ui.visuals().selection.bg_fill);
    ui.painter().hline(rect.x_range(), y, stroke);
}

/// 「元の並びで `insert_at` の手前」を、取り除いた後の添字へ直す。
///
/// 先に `remove(from)` すると `from` より後ろが 1 つ詰まるため、
/// 挿入位置がそれより後ろなら 1 引く。
fn reorder_target(from: usize, insert_at: usize) -> usize {
    if insert_at > from {
        insert_at - 1
    } else {
        insert_at
    }
}

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/waypoint.png"))
        .expect("埋め込みアイコンの読み込みに失敗")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

struct SettingsApp {
    config: Config,
    selected_menu: Vec<usize>,
    /// 選択中の行の添字集合。複数選択に対応する (FR-6)。
    selected_items: std::collections::BTreeSet<usize>,
    /// 直近でフォーカス/クリックした行。Shift 範囲選択の基準にし、
    /// 単一項目の操作 (Edit 詳細編集など) の対象にも使う。
    active_item: Option<usize>,
    /// Shift 範囲選択の起点。Ctrl/Shift を伴わない選択で更新する。
    selection_anchor: Option<usize>,
    draft: Option<ItemDraft>,
    batch_draft: Option<BatchDraft>,
    move_to_menu_draft: Option<MoveToMenuDraft>,
    variables_draft: Option<VariablesDraft>,
    trigger_draft: Option<TriggerDraft>,
    import_draft: Option<ImportDraft>,
    /// アプリ内クリップボード。Ctrl+C でコピーした項目を保持する。
    clipboard: Vec<Item>,
    add_pending: bool,
    move_pending: bool,
    delete_pending: bool,
    close_pending: bool,
    dirty: bool,
    load_error: Option<String>,
    status: Option<String>,
    /// この描画で一覧の行がキーボードフォーカスを持っているか。
    /// `Enter` / `Delete` を一覧の操作として扱ってよいかの判定に使う。
    focused_row: Option<usize>,
    /// 前の描画での `focused_row`。フォーカスが移った瞬間を見るために持つ。
    previous_focused_row: Option<usize>,
    /// 次の描画で選択行へフォーカスを移すか。
    /// 起動直後と、キーボードで選択を動かした直後に立てる。
    focus_selected_row: bool,
    /// クリックで選択を確定した直後、フォーカス変化検出による
    /// 単一選択への上書きを 1 回だけ抑止する (Ctrl/Shift クリック用) 。
    suppress_focus_select: bool,
}

impl SettingsApp {
    fn load() -> Self {
        let (config, load_error) = match waypoint::config::load() {
            LoadOutcome::Loaded(config) | LoadOutcome::Created(config) => (config, None),
            LoadOutcome::Failed(error) => (Config::default(), Some(error)),
        };
        let active_item = (!config.items.is_empty()).then_some(0);
        let selected_items = active_item.into_iter().collect();
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
            import_draft: None,
            clipboard: Vec::new(),
            add_pending: false,
            move_pending: false,
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
    fn select_single(&mut self, index: usize) {
        self.selected_items.clear();
        self.selected_items.insert(index);
        self.active_item = Some(index);
        self.selection_anchor = Some(index);
    }

    /// Ctrl+クリック相当。1 件だけ選択集合へ追加/除外する。
    fn toggle_selection(&mut self, index: usize) {
        if !self.selected_items.remove(&index) {
            self.selected_items.insert(index);
        }
        self.active_item = Some(index);
        self.selection_anchor = Some(index);
    }

    /// Shift+クリック / Shift+矢印相当。アンカーから index までを選択する。
    fn extend_selection_to(&mut self, index: usize) {
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

    fn select_all(&mut self) {
        let Some(len) = self.current_items().map(Vec::len) else {
            return;
        };
        self.selected_items = (0..len).collect();
        if self.active_item.is_none() {
            self.active_item = len.checked_sub(1);
        }
    }

    /// 選択集合を添字の昇順で返す。
    fn selected_indices(&self) -> Vec<usize> {
        self.selected_items.iter().copied().collect()
    }

    fn selection_summary(&self) -> String {
        let count = self.selected_items.len();
        if count <= 1 {
            "The selected item".to_string()
        } else {
            format!("The {count} selected items")
        }
    }

    fn current_items(&self) -> Option<&Vec<Item>> {
        items_at(&self.config, &self.selected_menu)
    }

    fn current_items_mut(&mut self) -> Option<&mut Vec<Item>> {
        items_at_mut(&mut self.config, &self.selected_menu)
    }

    fn begin_add(&mut self, kind: DraftKind) {
        self.draft = Some(ItemDraft::new(kind));
    }

    /// 単一選択なら詳細編集、複数選択ならバッチ編集ダイアログを開く。
    fn begin_edit(&mut self) {
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

    fn begin_batch_edit(&mut self) {
        if self.selected_items.len() < 2 {
            return;
        }
        self.batch_draft = Some(BatchDraft::default());
    }

    fn apply_draft(&mut self) {
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
    fn apply_batch_draft(&mut self) {
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
    fn insert_my_special_folders_preset(&mut self) {
        if let Some(items) = self.current_items_mut() {
            items.push(waypoint::config::my_special_folders_item());
            let index = items.len() - 1;
            self.select_single(index);
            self.dirty = true;
            self.status = None;
        }
    }

    /// 選択中の全項目を直後にまとめて複製する。
    fn duplicate_selected(&mut self) {
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
    fn remove_selected(&mut self) {
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
    }

    /// 選択中の全項目をまとめて上下へ動かす。連続していなくてもよい。
    ///
    /// 端 (先頭/末尾) に達した項目が一つでもあれば全体を動かさない。
    /// バラバラな選択でも相対順序を保ったまま 1 段ずつ動かす。
    fn move_selected(&mut self, delta: isize) {
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
    fn reorder(&mut self, from: usize, insert_at: usize) {
        let Some(items) = self.current_items_mut() else {
            return;
        };
        if from >= items.len() || insert_at > items.len() {
            return;
        }
        let target = reorder_target(from, insert_at);
        if target == from {
            return;
        }
        let item = items.remove(from);
        items.insert(target, item);
        self.select_single(target);
        self.dirty = true;
        self.status = None;
    }

    fn add_dropped_folders(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            if let Some(items) = self.current_items_mut() {
                items.push(Item::Folder {
                    name,
                    path: path.display().to_string(),
                    open: None,
                    icon: None,
                    show_branch: false,
                });
                let index = items.len() - 1;
                self.select_single(index);
                self.dirty = true;
                self.status = None;
            }
        }
    }

    /// 選択中の全項目をアプリ内クリップボードへコピーする。
    fn copy_selected(&mut self) {
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
    fn paste_clipboard(&mut self) {
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
    fn move_selected_to_menu(&mut self, target: &[usize]) {
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

    fn save(&mut self) -> bool {
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

    fn open_variables(&mut self) {
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

    fn open_trigger(&mut self) {
        self.trigger_draft = Some(TriggerDraft::from_config(&self.config));
    }

    fn open_import(&mut self) {
        self.import_draft = Some(ImportDraft {
            root: String::new(),
            depth: 2,
            preview: None,
            error: None,
        });
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        if self.dirty {
            self.close_pending = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// いずれかのダイアログ (モーダル相当) を表示中か。
    fn any_dialog_open(&self) -> bool {
        self.draft.is_some()
            || self.batch_draft.is_some()
            || self.move_to_menu_draft.is_some()
            || self.variables_draft.is_some()
            || self.trigger_draft.is_some()
            || self.import_draft.is_some()
            || self.add_pending
            || self.move_pending
            || self.delete_pending
            || self.close_pending
    }

    /// Shift+矢印での範囲選択伸縮。egui の標準フォーカス移動 (Tab 系) が
    /// 矢印キーを Shift 修飾ごと横取りする前に、一覧の描画より先に消費する。
    fn handle_range_selection_keys(&mut self, ctx: &egui::Context) {
        if self.any_dialog_open() {
            return;
        }
        let item_count = self.current_items().map_or(0, Vec::len);
        if item_count == 0 {
            return;
        }
        if consume_key_exact(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowUp) {
            let next = self.active_item.unwrap_or(0).saturating_sub(1);
            self.extend_selection_to(next);
            self.focus_selected_row = true;
        }
        if consume_key_exact(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowDown) {
            let next = (self.active_item.unwrap_or(0) + 1).min(item_count - 1);
            self.extend_selection_to(next);
            self.focus_selected_row = true;
        }
    }

    /// 画面全体のショートカット。
    ///
    /// **一覧を描いた後に呼ぶこと。** egui はフォーカス中のウィジェットを
    /// `Enter` で起動するが、その判定は `key_pressed` でイベントを読むだけなので、
    /// 描画前に `consume_key` すると打鍵がウィジェットへ届かなくなる。
    /// 先に消費していたため、Tab で送ったフォーカス先のボタンが Enter で
    /// 押せなくなっていた。
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.any_dialog_open() {
            return;
        }
        if consume_key_exact(ctx, egui::Modifiers::CTRL, egui::Key::S) {
            self.save();
        }
        if consume_key_exact(ctx, egui::Modifiers::CTRL, egui::Key::A) {
            self.select_all();
            self.focus_selected_row = true;
        }
        if consume_key_exact(ctx, egui::Modifiers::CTRL, egui::Key::C) {
            self.copy_selected();
        }
        if consume_key_exact(ctx, egui::Modifiers::CTRL, egui::Key::V) {
            self.paste_clipboard();
            self.focus_selected_row = true;
        }

        // どこにもフォーカスが無いと矢印キーは何も起こさない。egui は
        // フォーカス中のウィジェットを基準に次を探すため。Esc を押した後
        // (egui はフォーカスを外す) に一覧が反応しなくなるので、
        // 選択行へフォーカスを戻して続きから動かせるようにする
        let nothing_focused = ctx.memory(|memory| memory.focused().is_none());
        if nothing_focused
            && (consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::ArrowDown)
                || consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::ArrowUp))
        {
            self.focus_selected_row = true;
        }

        // Enter / Delete は一覧に対する操作。ボタンやコンボにフォーカスが
        // あるときはそちらの打鍵なので横取りしない
        let list_has_keys = self.focused_row.is_some() || !focus_takes_enter(ctx);
        if list_has_keys {
            if consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Enter) {
                self.begin_edit();
            }
            if consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Delete)
                && !self.selected_items.is_empty()
            {
                self.delete_pending = true;
            }
        }

        if consume_key_exact(ctx, egui::Modifiers::CTRL, egui::Key::D) {
            self.duplicate_selected();
            self.focus_selected_row = true;
        }
        if consume_key_exact(ctx, egui::Modifiers::ALT, egui::Key::ArrowUp) {
            self.move_selected(-1);
            self.focus_selected_row = true;
        }
        if consume_key_exact(ctx, egui::Modifiers::ALT, egui::Key::ArrowDown) {
            self.move_selected(1);
            self.focus_selected_row = true;
        }
    }

    fn show_items(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default().show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                        ui.close();
                    }
                    if ui.button("Save & Close").clicked() {
                        if self.save() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.close();
                    }
                    if ui.button("Close").clicked() {
                        self.request_close(&ctx);
                        ui.close();
                    }
                });
                ui.menu_button("Favorite", |ui| {
                    if ui.button("Add folder").clicked() {
                        self.begin_add(DraftKind::Folder);
                        ui.close();
                    }
                    if ui.button("Add special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
                        ui.close();
                    }
                    if ui.button("Add shell location").clicked() {
                        self.begin_add(DraftKind::Shell);
                        ui.close();
                    }
                    if ui.button("Add menu").clicked() {
                        self.begin_add(DraftKind::Submenu);
                        ui.close();
                    }
                    if ui.button("Add separator").clicked() {
                        self.begin_add(DraftKind::Separator);
                        ui.close();
                    }
                    if ui.button("Import folder structure...").clicked() {
                        self.open_import();
                        ui.close();
                    }
                    if ui.button("Add My Special Folders preset").clicked() {
                        self.insert_my_special_folders_preset();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(!self.selected_items.is_empty(), egui::Button::new("Edit"))
                        .clicked()
                    {
                        self.begin_edit();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.selected_items.is_empty(),
                            egui::Button::new("Move to menu..."),
                        )
                        .clicked()
                    {
                        self.move_to_menu_draft = Some(MoveToMenuDraft::default());
                        ui.close();
                    }
                });
                ui.menu_button("Options", |ui| {
                    if ui.button("Variables...").clicked() {
                        self.open_variables();
                        ui.close();
                    }
                    if ui.button("Trigger...").clicked() {
                        self.open_trigger();
                        ui.close();
                    }
                });
            });
            ui.separator();

            ui.label("Menu or group to edit:");
            let choices = menu_choices(&self.config);
            let selected_name = choices
                .iter()
                .find(|(path, _)| path == &self.selected_menu)
                .map(|(_, name)| name.as_str())
                .unwrap_or("Main");
            egui::ComboBox::from_id_salt("menu_to_edit")
                .selected_text(selected_name)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (path, name) in choices {
                        if ui
                            .selectable_label(path == self.selected_menu, name)
                            .clicked()
                        {
                            let first = items_at(&self.config, &path)
                                .and_then(|items| (!items.is_empty()).then_some(0));
                            self.selected_items = first.into_iter().collect();
                            self.active_item = first;
                            self.selection_anchor = first;
                            self.selected_menu = path;
                        }
                    }
                });
            ui.add_space(8.0);

            let rows = self.current_items().cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let selected = !self.selected_items.is_empty();
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Up").min_size([34.0, 28.0].into()),
                        )
                        .on_hover_text("Move up (Alt+Up)")
                        .clicked()
                    {
                        self.move_selected(-1);
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Down").min_size([34.0, 28.0].into()),
                        )
                        .on_hover_text("Move down (Alt+Down)")
                        .clicked()
                    {
                        self.move_selected(1);
                    }
                });

                let list_width = (ui.available_width() - 92.0).max(360.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(list_width, 400.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(list_width);
                        ui.set_max_width(list_width);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_size(egui::vec2(list_width - 4.0, 360.0));
                            egui::ScrollArea::both()
                                .min_scrolled_height(360.0)
                                .show(ui, |ui| {
                                    self.show_item_rows(ui, &rows);
                                });
                        });
                        ui.weak("Drag rows to reorder. Drop folders into the list to add them");
                    },
                );

                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 9.0;
                    if ui
                        .add(egui::Button::new("Add").min_size([74.0, 32.0].into()))
                        .clicked()
                    {
                        self.add_pending = true;
                    }
                    if ui
                        .add(egui::Button::new("Import").min_size([74.0, 32.0].into()))
                        .on_hover_text("Import a folder structure")
                        .clicked()
                    {
                        self.open_import();
                    }
                    let selected = !self.selected_items.is_empty();
                    let multi = self.selected_items.len() > 1;
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new(if multi { "Edit all..." } else { "Edit" })
                                .min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text(if multi {
                            "Edit shared properties (Open mode, Show Git branch) for all selected items"
                        } else {
                            "Edit this item"
                        })
                        .clicked()
                    {
                        self.begin_edit();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Remove").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Remove selected items (Delete)")
                        .clicked()
                    {
                        self.delete_pending = true;
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Duplicate").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Duplicate selected items (Ctrl+D)")
                        .clicked()
                    {
                        self.duplicate_selected();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Copy").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Copy selected items to clipboard (Ctrl+C)")
                        .clicked()
                    {
                        self.copy_selected();
                    }
                    if ui
                        .add_enabled(
                            !self.clipboard.is_empty(),
                            egui::Button::new("Paste").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Paste clipboard items here (Ctrl+V)")
                        .clicked()
                    {
                        self.paste_clipboard();
                    }
                    if ui
                        .add_enabled(
                            selected,
                            egui::Button::new("Move").min_size([74.0, 32.0].into()),
                        )
                        .on_hover_text("Move up/down or to another menu")
                        .clicked()
                    {
                        self.move_pending = true;
                    }
                });
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let button_widths = 120.0 + 90.0 + 90.0 + ui.spacing().item_spacing.x * 2.0;
                ui.add_space(((ui.available_width() - button_widths) / 2.0).max(0.0));
                if ui
                    .add_enabled(
                        self.load_error.is_none(),
                        egui::Button::new("Save & Close").min_size([120.0, 34.0].into()),
                    )
                    .clicked()
                    && self.save()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui
                    .add_enabled(
                        self.load_error.is_none(),
                        egui::Button::new("Save").min_size([90.0, 34.0].into()),
                    )
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add(egui::Button::new("Close").min_size([90.0, 34.0].into()))
                    .clicked()
                {
                    self.request_close(&ctx);
                }
            });
            ui.horizontal_centered(|ui| {
                if self.dirty {
                    ui.weak("Unsaved changes");
                } else if let Some(status) = &self.status {
                    ui.label(status);
                }
            });

            if let Some(error) = &self.load_error {
                ui.colored_label(
                    egui::Color32::RED,
                    format!("Configuration could not be loaded: {error}"),
                );
                ui.label("The file was not overwritten. Close this window and repair config.json.");
            }
        });
    }

    /// 項目一覧の各行を描く。行全体がドラッグ元になる (FR-6.3) 。
    ///
    /// `Grid` ではなく行ごとの `horizontal` にしているのは、
    /// 行全体を 1 つのドラッグ元として掴めるようにするため。
    fn show_item_rows(&mut self, ui: &mut egui::Ui, rows: &[Item]) {
        ui.horizontal(|ui| {
            ui.add_space(ROW_INDENT);
            ui.allocate_ui_with_layout(
                egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.strong("Name"),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(COL_KIND, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.strong("Type"),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(COL_OPEN, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.strong("Open"),
            );
            ui.strong("Location or content");
        });
        ui.separator();

        // フォーカスは毎回描画から拾い直す
        self.focused_row = None;

        // ドラッグ中に「何番目の手前へ入るか」。行の上半分/下半分で決める
        let mut insert_at: Option<usize> = None;
        let mut dropped: Option<(usize, usize)> = None;

        for (index, item) in rows.iter().enumerate() {
            let id = egui::Id::new(("item_row", &self.selected_menu, index));
            let response = ui
                .dnd_drag_source(id, DragRow { index }, |ui| {
                    self.show_item_row(ui, index, item, id);
                })
                .response;

            // 行全体 (ドラッグ元) をキーボードフォーカスの単位にする。
            // `dnd_drag_source` は `Sense::drag()` で掴むが、これは
            // `FOCUSABLE` を含むのでフォーカス候補になる。矢印キーは
            // この矩形を基準に隣の行を探す
            if response.has_focus() {
                self.focused_row = Some(index);
            }
            if self.focus_selected_row && self.active_item == Some(index) {
                response.request_focus();
                self.focus_selected_row = false;
            }

            // 掴んでいる行の上半分なら手前、下半分なら後ろへ挿入する
            if let Some(pointer) = ui.ctx().pointer_interact_pos()
                && response.dnd_hover_payload::<DragRow>().is_some()
            {
                let rect = response.rect;
                let after = pointer.y > rect.center().y;
                let at = if after { index + 1 } else { index };
                insert_at = Some(at);
                draw_insert_line(ui, rect, after);
            }

            if let Some(payload) = response.dnd_release_payload::<DragRow>() {
                let at = insert_at.unwrap_or(index);
                dropped = Some((payload.index, at));
            }
        }

        // 一覧の末尾へ落とせるように、残り領域も受け口にする
        let tail = ui.allocate_response(
            egui::vec2(
                ui.available_width(),
                TAIL_DROP_HEIGHT.max(ui.available_height()),
            ),
            egui::Sense::hover(),
        );
        if tail.dnd_hover_payload::<DragRow>().is_some() {
            draw_insert_line(ui, tail.rect, false);
        }
        if let Some(payload) = tail.dnd_release_payload::<DragRow>() {
            dropped = Some((payload.index, rows.len()));
        }

        // 矢印キーは egui のフォーカスを行から行へ動かす。選択を追従させないと
        // 網掛けも右側のボタン (Edit / Remove など) も付いてこない。
        //
        // `gained_focus()` は使えない。egui は矢印での移動を `end_pass` で
        // 差し替えるため、次の描画では「前フレームから持っていた」扱いになり
        // 一度も立たない (`memory/mod.rs` の `end_pass`) 。自前で変化を見る
        if let Some(row) = self.focused_row
            && self.focused_row != self.previous_focused_row
        {
            if self.suppress_focus_select {
                self.suppress_focus_select = false;
            } else {
                self.select_single(row);
            }
        }
        self.previous_focused_row = self.focused_row;

        if let Some((from, at)) = dropped {
            self.reorder(from, at);
        }
    }

    /// 1 行分のセルを描く。列幅は見出しと揃える。
    ///
    /// `row_id` は行 (ドラッグ元) の ID。名前セルもフォーカスを取れてしまうため、
    /// フォーカスが来たら行へ渡し直す。
    fn show_item_row(&mut self, ui: &mut egui::Ui, index: usize, item: &Item, row_id: egui::Id) {
        ui.horizontal(|ui| {
            // 掴む場所が分かるようにグリップを置く
            ui.add_space(2.0);
            ui.weak("⠿");

            let selected = self.selected_items.contains(&index);
            let response = ui.allocate_ui_with_layout(
                egui::vec2(COL_NAME, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.selectable_label(selected, item.label().unwrap_or("----------------")),
            );
            let label = response.inner;
            // 名前セルもフォーカスを取れる。行と 2 つ並ぶと矢印キーが行の中で
            // 足踏みし、そのまま一覧の外のボタンへ抜けてしまう。
            // フォーカスは行へ寄せて、1 行 1 つに保つ
            if label.has_focus() {
                ui.memory_mut(|memory| memory.request_focus(row_id));
                ui.ctx().request_repaint();
            }
            if label.clicked() {
                // Ctrl/Shift 修飾でトグル選択・範囲選択を切り替える (FR-6)
                let modifiers = ui.ctx().input(|input| input.modifiers);
                if modifiers.command {
                    self.toggle_selection(index);
                } else if modifiers.shift {
                    self.extend_selection_to(index);
                } else {
                    self.select_single(index);
                }
                // クリックによる選択はここで確定済みなので、後続のフォーカス変化
                // 検出 (矢印キー用) がこの行を単一選択で上書きしないようにする
                self.suppress_focus_select = true;
            }
            if label.double_clicked() {
                self.select_single(index);
                self.begin_edit();
            }

            ui.allocate_ui_with_layout(
                egui::vec2(COL_KIND, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(item_kind(item)),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(COL_OPEN, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(item_open(item)),
            );
            ui.label(item_detail(item));
        });
    }

    fn show_item_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new(if draft.editing.is_some() {
            "Edit item"
        } else {
            "Add item"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            match draft.kind {
                DraftKind::Folder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Path");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut draft.path);
                        if ui.button("Browse...").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            if draft.name.is_empty()
                                && let Some(name) = path.file_name()
                            {
                                draft.name = name.to_string_lossy().into_owned();
                            }
                            draft.path = path.display().to_string();
                        }
                    });
                    show_open_mode(ui, &mut draft.open);
                    ui.checkbox(&mut draft.show_branch, "Show Git branch name")
                        .on_hover_text(
                            "Appends [branch] to the menu label when the path is inside a Git work tree.",
                        );
                }
                DraftKind::SpecialFolder => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Known folder");
                    egui::ComboBox::from_id_salt("known_folder")
                        .selected_text(&draft.known_folder)
                        .show_ui(ui, |ui| {
                            for name in waypoint::known_folder::NAMES {
                                ui.selectable_value(
                                    &mut draft.known_folder,
                                    (*name).to_string(),
                                    *name,
                                );
                            }
                        });
                    show_open_mode(ui, &mut draft.open);
                }
                DraftKind::Shell => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.label("Target (e.g. shell:MyComputerFolder)");
                    ui.text_edit_singleline(&mut draft.target);
                }
                DraftKind::Submenu => {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.checkbox(&mut draft.show_branch, "Show Git branch name for folders inside")
                        .on_hover_text(
                            "Applies showBranch to every Folder item nested under this menu, without editing them individually.",
                        );
                }
                DraftKind::Separator => {
                    ui.label("Heading (optional)");
                    ui.text_edit_singleline(&mut draft.name);
                }
            }

            if let Some(error) = &draft.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                apply = ui.button("OK").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        lock_modal_focus(ctx, &window);

        // 単一行の入力欄しか無いので Enter を確定に使える
        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            if let Some(error) = draft.validate() {
                draft.error = Some(error);
            } else {
                self.apply_draft();
            }
        } else if cancel || dismiss {
            self.draft = None;
        }
    }

    fn show_variables_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.variables_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let mut remove = None;
        let window = egui::Window::new("Variables")
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("variables_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Value");
                        ui.end_row();
                        for (index, (name, value)) in draft.entries.iter_mut().enumerate() {
                            ui.text_edit_singleline(name);
                            ui.text_edit_singleline(value);
                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if ui.button("Add variable").clicked() {
                    draft.entries.push((String::new(), String::new()));
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        if let Some(index) = remove {
            draft.entries.remove(index);
        }
        // 名前と値はどちらも単一行なので Enter を確定に使える
        let (accept, dismiss) = dialog_keys(ctx, true);
        let cancel = cancel || dismiss;
        if apply || accept {
            let mut variables = std::collections::BTreeMap::new();
            let mut error = None;
            for (name, value) in &draft.entries {
                let name = name.trim();
                if name.is_empty() {
                    error = Some("Variable name is required.".to_string());
                    break;
                }
                if variables.insert(name.to_string(), value.clone()).is_some() {
                    error = Some(format!("Variable name is duplicated: {name}"));
                    break;
                }
            }
            if let Some(error) = error {
                draft.error = Some(error);
            } else {
                self.config.variables = variables;
                self.variables_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.variables_draft = None;
        }
    }

    /// 選択中の複数項目に共通するプロパティをまとめて設定するダイアログ (FR-6) 。
    /// 各欄は 3 状態 (変更しない / 値A / 値B) で、既定は「変更しない」。
    fn show_batch_editor(&mut self, ctx: &egui::Context) {
        let selected_count = self.selected_items.len();
        let Some(draft) = self.batch_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Edit selected items")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Applies to {selected_count} (Folder / Special folder only where relevant)."
                ));
                ui.separator();
                ui.label("Open mode");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut draft.open, None, "Don't change");
                    ui.selectable_value(&mut draft.open, Some(OpenMode::NewWindow), "New window");
                    ui.selectable_value(
                        &mut draft.open,
                        Some(OpenMode::Reuse),
                        "Reuse Explorer window",
                    );
                });
                ui.label("Show Git branch name");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut draft.show_branch, None, "Don't change");
                    ui.selectable_value(&mut draft.show_branch, Some(true), "On");
                    ui.selectable_value(&mut draft.show_branch, Some(false), "Off");
                });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            self.apply_batch_draft();
        } else if cancel || dismiss {
            self.batch_draft = None;
        }
    }

    /// 選択中の項目を別メニューへまとめて移すダイアログ (FR-6.3) 。
    fn show_move_to_menu_editor(&mut self, ctx: &egui::Context) {
        let prompt = format!("Move {} to:", self.selection_summary().to_ascii_lowercase());
        let Some(draft) = self.move_to_menu_draft.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let choices = menu_choices(&self.config);
        let selected_name = choices
            .iter()
            .find(|(path, _)| path == &draft.target)
            .map(|(_, name)| name.as_str())
            .unwrap_or("Main");
        let window = egui::Window::new("Move to menu")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&prompt);
                egui::ComboBox::from_id_salt("move_to_menu_target")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (path, name) in &choices {
                            ui.selectable_value(&mut draft.target, path.clone(), name);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Move").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        let (accept, dismiss) = dialog_keys(ctx, true);
        if apply || accept {
            let target = draft.target.clone();
            self.move_selected_to_menu(&target);
            self.move_to_menu_draft = None;
        } else if cancel || dismiss {
            self.move_to_menu_draft = None;
        }
    }

    fn show_trigger_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.trigger_draft.as_mut() else {
            return;
        };
        poll_hotkey_capture(ctx, draft);
        let mut apply = false;
        let mut cancel = false;
        let window = egui::Window::new("Trigger")
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.checkbox(&mut draft.middle_click, "Enable middle click");
                ui.label("Hotkey");
                hotkey_row(ui, draft, HotkeyField::Menu);
                ui.weak("Example: Win+W");
                ui.separator();
                ui.strong("Quick Launch");
                ui.label("Hotkey");
                hotkey_row(ui, draft, HotkeyField::QuickLaunch);
                ui.checkbox(&mut draft.include_recent_folders, "Include Recent Folders");
                ui.checkbox(
                    &mut draft.include_frequent_folders,
                    "Include Frequent Folders",
                );
                ui.checkbox(&mut draft.include_open_windows, "Include Open Windows");
                ui.checkbox(
                    &mut draft.include_bookmarks,
                    "Include browser bookmarks (type \"b \" to search)",
                );
                ui.checkbox(
                    &mut draft.include_apps,
                    "Include installed apps (type \"a \" to search)",
                );
                ui.checkbox(
                    &mut draft.include_everything,
                    "Search all files via Everything (type \"f \" to search; requires Everything to be running)",
                );
                ui.checkbox(&mut draft.search_paths, "Search folder paths");
                ui.horizontal(|ui| {
                    ui.label("Visible results");
                    ui.add(egui::DragValue::new(&mut draft.visible_results).range(12..=24));
                });
                ui.separator();
                ui.label("Excluded processes (one per line)");
                ui.add(
                    egui::TextEdit::multiline(&mut draft.excluded_processes)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY),
                );
                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        // 除外プロセス欄が複数行なので Enter は改行に譲り、Esc だけ受ける。
        // 記録中の打鍵はフックが握り潰すため、ここには Esc は届かない
        let (_, dismiss) = dialog_keys(ctx, false);
        let cancel = cancel || dismiss;
        if apply || cancel {
            // 画面を閉じるならフックを残さない
            hotkey_capture::stop();
            draft.recording = None;
        }
        if apply {
            if waypoint::trigger::parse_hotkey(draft.hotkey.trim()).is_none() {
                draft.error = Some("Hotkey is invalid.".to_string());
            } else if waypoint::trigger::parse_hotkey(draft.quick_launch_hotkey.trim()).is_none() {
                draft.error = Some("Quick Launch hotkey is invalid.".to_string());
            } else if draft
                .hotkey
                .trim()
                .eq_ignore_ascii_case(draft.quick_launch_hotkey.trim())
            {
                draft.error = Some("The two hotkeys must be different.".to_string());
            } else {
                self.config.settings.trigger.middle_click = draft.middle_click;
                self.config.settings.trigger.hotkey = draft.hotkey.trim().to_string();
                self.config.settings.trigger.excluded_processes = draft
                    .excluded_processes
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                self.config.settings.quick_launch.hotkey =
                    draft.quick_launch_hotkey.trim().to_string();
                self.config.settings.quick_launch.include_recent_folders =
                    draft.include_recent_folders;
                self.config.settings.quick_launch.include_frequent_folders =
                    draft.include_frequent_folders;
                self.config.settings.quick_launch.include_open_windows = draft.include_open_windows;
                self.config.settings.quick_launch.include_bookmarks = draft.include_bookmarks;
                self.config.settings.quick_launch.include_apps = draft.include_apps;
                self.config.settings.quick_launch.include_everything = draft.include_everything;
                self.config.settings.quick_launch.search_paths = draft.search_paths;
                self.config.settings.quick_launch.visible_results =
                    draft.visible_results.clamp(12, 24);
                self.trigger_draft = None;
                self.dirty = true;
                self.status = None;
            }
        } else if cancel {
            self.trigger_draft = None;
        }
    }

    fn show_import_editor(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.import_draft.as_mut() else {
            return;
        };
        let mut refresh = false;
        let mut apply = false;
        let mut cancel = false;

        let window = egui::Window::new("Import folder structure")
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Root folder");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut draft.root).desired_width(460.0));
                    if ui.button("Browse...").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        draft.root = path.display().to_string();
                        refresh = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Subfolder depth");
                    ui.add(egui::DragValue::new(&mut draft.depth).range(0..=10));
                    if ui.button("Preview").clicked() {
                        refresh = true;
                    }
                });
                ui.weak("Depth 0 imports only the selected folder. This is a one-time import.");

                if let Some(error) = &draft.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.separator();
                if let Some(preview) = draft.preview.as_mut() {
                    ui.label(format!(
                        "Preview: {} selected folder(s). Rename or clear items before importing.",
                        preview.included_count()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(330.0)
                        .show(ui, |ui| show_import_node(ui, preview, 0));
                } else {
                    ui.weak("Choose a folder and select Preview.");
                }

                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(
                            draft
                                .preview
                                .as_ref()
                                .is_some_and(|preview| preview.included_count() > 0),
                            egui::Button::new("Import"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        lock_modal_focus(ctx, &window);

        if refresh {
            let root = std::path::Path::new(draft.root.trim());
            match waypoint::folder_import::scan(root, draft.depth) {
                Ok(preview) => {
                    draft.preview = Some(preview);
                    draft.error = None;
                }
                Err(error) => {
                    draft.preview = None;
                    draft.error = Some(format!("Could not read folder: {error}"));
                }
            }
        }

        // 取り込みは Preview が済んでいるときだけ Enter で確定できる
        let (accept, dismiss) = dialog_keys(ctx, true);
        let apply = apply
            || (accept
                && draft
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.included_count() > 0));
        let cancel = cancel || dismiss;
        if apply {
            let item = self
                .import_draft
                .as_ref()
                .and_then(|draft| draft.preview.as_ref())
                .and_then(waypoint::folder_import::FolderNode::to_item);
            if let Some(item) = item {
                if let Some(items) = self.current_items_mut() {
                    items.push(item);
                    let index = items.len() - 1;
                    self.select_single(index);
                    self.dirty = true;
                    self.status = None;
                }
                self.import_draft = None;
            }
        } else if cancel {
            self.import_draft = None;
        }
    }

    fn show_confirmations(&mut self, ctx: &egui::Context) {
        if self.add_pending {
            let window = egui::Window::new("Add favorite")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if ui.button("Folder").clicked() {
                        self.begin_add(DraftKind::Folder);
                        self.add_pending = false;
                    }
                    if ui.button("Special folder").clicked() {
                        self.begin_add(DraftKind::SpecialFolder);
                        self.add_pending = false;
                    }
                    if ui.button("Shell location").clicked() {
                        self.begin_add(DraftKind::Shell);
                        self.add_pending = false;
                    }
                    if ui.button("Menu").clicked() {
                        self.begin_add(DraftKind::Submenu);
                        self.add_pending = false;
                    }
                    if ui.button("Separator").clicked() {
                        self.begin_add(DraftKind::Separator);
                        self.add_pending = false;
                    }
                    if ui.button("Folder structure...").clicked() {
                        self.open_import();
                        self.add_pending = false;
                    }
                    if ui.button("My Special Folders preset").clicked() {
                        self.insert_my_special_folders_preset();
                        self.add_pending = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.add_pending = false;
                    }
                });
            lock_modal_focus(ctx, &window);
            // 種類を選ぶ画面で既定の 1 つは決められない。Esc の取り消しだけ受ける
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.add_pending = false;
            }
        }

        if self.move_pending {
            let window = egui::Window::new("Move favorite")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Move up").clicked() {
                            self.move_selected(-1);
                            self.move_pending = false;
                        }
                        if ui.button("Move down").clicked() {
                            self.move_selected(1);
                            self.move_pending = false;
                        }
                        if ui.button("Move to menu...").clicked() {
                            self.move_to_menu_draft = Some(MoveToMenuDraft::default());
                            self.move_pending = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.move_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // 上下どちらも既定にはできない。Esc の取り消しだけ受ける
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.move_pending = false;
            }
        }

        if self.delete_pending {
            let window = egui::Window::new("Remove item?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("{} will be removed.", self.selection_summary()));
                    ui.horizontal(|ui| {
                        if ui.button("Remove").clicked() {
                            self.remove_selected();
                            self.delete_pending = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.delete_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // Delete で出した確認なので Enter は削除に割り当てる
            let (accept, dismiss) = dialog_keys(ctx, true);
            if accept {
                self.remove_selected();
                self.delete_pending = false;
            } else if dismiss {
                self.delete_pending = false;
            }
        }

        if self.close_pending {
            let window = egui::Window::new("Discard changes?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("There are unsaved changes.");
                    ui.horizontal(|ui| {
                        if ui.button("Discard").clicked() {
                            self.dirty = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.close_pending = false;
                        }
                    });
                });
            lock_modal_focus(ctx, &window);
            // 破棄は取り返しがつかない。Enter には割り当てず、Esc で編集へ戻す
            let (_, dismiss) = dialog_keys(ctx, false);
            if dismiss {
                self.close_pending = false;
            }
        }
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
        self.show_import_editor(&ctx);
        self.show_confirmations(&ctx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DraftKind {
    Folder,
    SpecialFolder,
    Shell,
    Submenu,
    Separator,
}

struct ItemDraft {
    editing: Option<usize>,
    kind: DraftKind,
    name: String,
    path: String,
    known_folder: String,
    target: String,
    open: OpenMode,
    icon: Option<String>,
    show_branch: bool,
    submenu_items: Vec<Item>,
    error: Option<String>,
}

struct VariablesDraft {
    entries: Vec<(String, String)>,
    error: Option<String>,
}

/// 複数項目の共有プロパティをまとめて編集するダイアログ (FR-6) 。
/// `None` は「変更しない」を表す。
#[derive(Default)]
struct BatchDraft {
    open: Option<OpenMode>,
    show_branch: Option<bool>,
}

/// 選択項目をまとめて別メニューへ移す先を選ぶダイアログ (FR-6.3) 。
#[derive(Default)]
struct MoveToMenuDraft {
    target: Vec<usize>,
}

/// ホットキー 1 欄。直接入力と、実際のキー入力からの記録 (FR-6.8.1) 。
fn hotkey_row(ui: &mut egui::Ui, draft: &mut TriggerDraft, field: HotkeyField) {
    let recording = draft.recording == Some(field);
    ui.horizontal(|ui| {
        ui.add_enabled(
            !recording,
            egui::TextEdit::singleline(draft.field_mut(field)).desired_width(200.0),
        );
        let label = if recording { "Press keys..." } else { "Record" };
        if ui.button(label).clicked() {
            if recording {
                hotkey_capture::stop();
                draft.recording = None;
            } else if hotkey_capture::start() {
                draft.recording = Some(field);
                draft.error = None;
            } else {
                draft.error = Some("Could not capture keys. Type the hotkey instead.".to_string());
            }
        }
        if recording {
            ui.weak("Esc to cancel");
        }
    });
}

/// 記録中は毎フレーム結果を拾う。ウィンドウがフォーカスを失ったら、
/// 打鍵を握り潰したままにしないよう記録を打ち切る。
fn poll_hotkey_capture(ctx: &egui::Context, draft: &mut TriggerDraft) {
    let Some(field) = draft.recording else {
        return;
    };
    if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
        hotkey_capture::stop();
        draft.recording = None;
        return;
    }
    // フックの結果は egui のイベントで届かないので、記録中は描画を回し続ける
    ctx.request_repaint();
    match hotkey_capture::poll() {
        Some(hotkey_capture::Captured::Spec(spec)) => {
            *draft.field_mut(field) = spec;
            draft.recording = None;
        }
        Some(hotkey_capture::Captured::Cancelled) => draft.recording = None,
        Some(hotkey_capture::Captured::Unsupported) => {
            draft.error = Some("That key cannot be used. Use A-Z, 0-9 or F1-F24.".to_string());
            draft.recording = None;
        }
        None => {}
    }
}

struct TriggerDraft {
    middle_click: bool,
    hotkey: String,
    excluded_processes: String,
    quick_launch_hotkey: String,
    include_recent_folders: bool,
    include_frequent_folders: bool,
    include_open_windows: bool,
    include_bookmarks: bool,
    include_apps: bool,
    include_everything: bool,
    search_paths: bool,
    visible_results: usize,
    error: Option<String>,
    /// キー入力から記録中の欄 (FR-6.8.1) 。
    recording: Option<HotkeyField>,
}

/// ホットキーを持つ欄。記録先の指定に使う。
#[derive(Clone, Copy, PartialEq, Eq)]
enum HotkeyField {
    Menu,
    QuickLaunch,
}

struct ImportDraft {
    root: String,
    depth: usize,
    preview: Option<waypoint::folder_import::FolderNode>,
    error: Option<String>,
}

impl TriggerDraft {
    fn from_config(config: &Config) -> Self {
        let trigger = &config.settings.trigger;
        let quick_launch = &config.settings.quick_launch;
        Self {
            middle_click: trigger.middle_click,
            hotkey: trigger.hotkey.clone(),
            excluded_processes: trigger.excluded_processes.join("\n"),
            quick_launch_hotkey: quick_launch.hotkey.clone(),
            include_recent_folders: quick_launch.include_recent_folders,
            include_frequent_folders: quick_launch.include_frequent_folders,
            include_open_windows: quick_launch.include_open_windows,
            include_bookmarks: quick_launch.include_bookmarks,
            include_apps: quick_launch.include_apps,
            include_everything: quick_launch.include_everything,
            search_paths: quick_launch.search_paths,
            visible_results: quick_launch.visible_results,
            error: None,
            recording: None,
        }
    }
}

impl TriggerDraft {
    fn field_mut(&mut self, field: HotkeyField) -> &mut String {
        match field {
            HotkeyField::Menu => &mut self.hotkey,
            HotkeyField::QuickLaunch => &mut self.quick_launch_hotkey,
        }
    }
}

impl ItemDraft {
    fn new(kind: DraftKind) -> Self {
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

    fn from_item(index: usize, item: &Item) -> Self {
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

    fn validate(&self) -> Option<String> {
        match self.kind {
            DraftKind::Folder if self.name.trim().is_empty() => {
                Some("Name is required.".to_string())
            }
            DraftKind::Folder if self.path.trim().is_empty() => {
                Some("Path is required.".to_string())
            }
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

    fn into_item(self) -> Item {
        let open = (self.open != OpenMode::default()).then_some(self.open);
        match self.kind {
            DraftKind::Folder => Item::Folder {
                name: self.name,
                path: self.path,
                open,
                icon: self.icon,
                show_branch: self.show_branch,
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

fn items_at<'a>(config: &'a Config, path: &[usize]) -> Option<&'a Vec<Item>> {
    let mut items = &config.items;
    for &index in path {
        match items.get(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

fn items_at_mut<'a>(config: &'a mut Config, path: &[usize]) -> Option<&'a mut Vec<Item>> {
    let mut items = &mut config.items;
    for &index in path {
        match items.get_mut(index)? {
            Item::Submenu { items: child, .. } => items = child,
            _ => return None,
        }
    }
    Some(items)
}

fn menu_choices(config: &Config) -> Vec<(Vec<usize>, String)> {
    let mut choices = vec![(Vec::new(), "Main".to_string())];
    collect_menu_choices(&config.items, &mut Vec::new(), "", &mut choices);
    choices
}

fn collect_menu_choices(
    items: &[Item],
    path: &mut Vec<usize>,
    parent_name: &str,
    choices: &mut Vec<(Vec<usize>, String)>,
) {
    for (index, item) in items.iter().enumerate() {
        let Item::Submenu {
            name,
            items: children,
            ..
        } = item
        else {
            continue;
        };
        path.push(index);
        let full_name = if parent_name.is_empty() {
            name.clone()
        } else {
            format!("{parent_name} > {name}")
        };
        choices.push((path.clone(), full_name.clone()));
        collect_menu_choices(children, path, &full_name, choices);
        path.pop();
    }
}

fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::Folder { .. } => "Folder",
        Item::SpecialFolder { .. } => "Special folder",
        Item::Shell { .. } => "Shell location",
        Item::Submenu { .. } => "Submenu",
        Item::Separator { .. } => "Separator",
    }
}

fn item_detail(item: &Item) -> &str {
    match item {
        Item::Folder { path, .. } => path,
        Item::SpecialFolder { known_folder, .. } => known_folder,
        Item::Shell { target, .. } => target,
        Item::Submenu { .. } | Item::Separator { .. } => "",
    }
}

fn item_open(item: &Item) -> &'static str {
    match item {
        Item::Folder { open, .. } | Item::SpecialFolder { open, .. } => {
            match open.unwrap_or_default() {
                OpenMode::NewWindow => "New window",
                OpenMode::Reuse => "Reuse",
            }
        }
        Item::Shell { .. } | Item::Submenu { .. } | Item::Separator { .. } => "—",
    }
}

fn show_open_mode(ui: &mut egui::Ui, mode: &mut OpenMode) {
    ui.label("Open");
    ui.horizontal(|ui| {
        ui.radio_value(mode, OpenMode::NewWindow, "New window");
        ui.radio_value(mode, OpenMode::Reuse, "Reuse Explorer window");
    });
}

fn show_import_node(
    ui: &mut egui::Ui,
    node: &mut waypoint::folder_import::FolderNode,
    depth: usize,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        ui.checkbox(&mut node.included, "");
        ui.add(egui::TextEdit::singleline(&mut node.name).desired_width(360.0));
    });
    ui.add_enabled_ui(node.included, |ui| {
        for child in &mut node.children {
            show_import_node(ui, child, depth + 1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_item_path_resolves_submenu() {
        let mut config = Config {
            items: vec![Item::Submenu {
                name: "Tools".to_string(),
                items: vec![Item::Separator { name: None }],
                show_branch: false,
            }],
            ..Default::default()
        };

        items_at_mut(&mut config, &[0])
            .unwrap()
            .push(Item::Separator {
                name: Some("Build".to_string()),
            });

        assert_eq!(items_at(&config, &[0]).unwrap().len(), 2);
    }

    #[test]
    fn editing_submenu_preserves_its_children() {
        let original = Item::Submenu {
            name: "Old".to_string(),
            items: vec![Item::Separator { name: None }],
            show_branch: false,
        };
        let mut draft = ItemDraft::from_item(0, &original);
        draft.name = "New".to_string();

        assert_eq!(
            draft.into_item(),
            Item::Submenu {
                name: "New".to_string(),
                items: vec![Item::Separator { name: None }],
                show_branch: false,
            }
        );
    }

    /// 実際に remove → insert したときの並びを返す。
    /// `reorder_target` の補正が正しいことを、結果の並びで確かめる。
    fn reordered(len: usize, from: usize, insert_at: usize) -> Vec<usize> {
        let mut items: Vec<usize> = (0..len).collect();
        let item = items.remove(from);
        items.insert(reorder_target(from, insert_at), item);
        items
    }

    /// 後ろへ動かすとき、取り除きで添字が 1 つ詰まる分を補正する。
    #[test]
    fn dragging_down_lands_before_the_target() {
        // [0,1,2,3,4] の 1 を「3 の手前」へ → [0,2,1,3,4]
        assert_eq!(reordered(5, 1, 3), vec![0, 2, 1, 3, 4]);
    }

    /// 前へ動かすときは補正しない。
    #[test]
    fn dragging_up_lands_at_the_target() {
        // [0,1,2,3,4] の 3 を「1 の手前」へ → [0,3,1,2,4]
        assert_eq!(reordered(5, 3, 1), vec![0, 3, 1, 2, 4]);
    }

    /// 一覧の末尾へ落とすと最後尾に来る。
    #[test]
    fn dragging_to_the_tail_lands_last() {
        assert_eq!(reordered(4, 0, 4), vec![1, 2, 3, 0]);
    }

    /// 自分の位置とその直後は、どちらも並びを変えない。
    #[test]
    fn dropping_onto_itself_keeps_the_order() {
        assert_eq!(reordered(4, 2, 2), vec![0, 1, 2, 3]);
        assert_eq!(reordered(4, 2, 3), vec![0, 1, 2, 3]);
    }

    /// テスト用に `n` 個の Folder 項目を持つ最小の SettingsApp を組み立てる。
    fn app_with_folders(n: usize) -> SettingsApp {
        let items = (0..n)
            .map(|i| Item::Folder {
                name: format!("f{i}"),
                path: format!("C:/f{i}"),
                open: None,
                icon: None,
                show_branch: false,
            })
            .collect();
        SettingsApp {
            config: Config {
                items,
                ..Default::default()
            },
            selected_menu: Vec::new(),
            selected_items: std::collections::BTreeSet::new(),
            active_item: None,
            selection_anchor: None,
            draft: None,
            batch_draft: None,
            move_to_menu_draft: None,
            variables_draft: None,
            trigger_draft: None,
            import_draft: None,
            clipboard: Vec::new(),
            add_pending: false,
            move_pending: false,
            delete_pending: false,
            close_pending: false,
            dirty: false,
            load_error: None,
            status: None,
            focused_row: None,
            previous_focused_row: None,
            focus_selected_row: false,
            suppress_focus_select: false,
        }
    }

    fn names(app: &SettingsApp) -> Vec<String> {
        app.config
            .items
            .iter()
            .map(|item| item.label().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn extend_selection_to_covers_the_range_from_the_anchor() {
        let mut app = app_with_folders(5);
        app.select_single(1);
        app.extend_selection_to(3);
        assert_eq!(app.selected_items, [1, 2, 3].into_iter().collect());

        // アンカーは 1 のまま。逆方向へ伸ばすと 1 側の選択に切り替わる
        app.extend_selection_to(0);
        assert_eq!(app.selected_items, [0, 1].into_iter().collect());
    }

    #[test]
    fn toggle_selection_adds_and_removes_a_single_item() {
        let mut app = app_with_folders(3);
        app.select_single(0);
        app.toggle_selection(2);
        assert_eq!(app.selected_items, [0, 2].into_iter().collect());
        app.toggle_selection(0);
        assert_eq!(app.selected_items, [2].into_iter().collect());
    }

    #[test]
    fn remove_selected_removes_every_selected_item_at_once() {
        let mut app = app_with_folders(5);
        app.selected_items = [0, 2, 4].into_iter().collect();
        app.remove_selected();
        assert_eq!(names(&app), vec!["f1", "f3"]);
    }

    #[test]
    fn move_selected_shifts_a_scattered_selection_up_together() {
        let mut app = app_with_folders(5);
        // f0 f1 f2 f3 f4 のうち f1, f3 を選び上へ動かす
        app.selected_items = [1, 3].into_iter().collect();
        app.move_selected(-1);
        assert_eq!(names(&app), vec!["f1", "f0", "f3", "f2", "f4"]);
        assert_eq!(app.selected_items, [0, 2].into_iter().collect());
    }

    #[test]
    fn move_selected_does_nothing_when_any_item_is_already_at_the_edge() {
        let mut app = app_with_folders(4);
        app.selected_items = [0, 2].into_iter().collect();
        app.move_selected(-1);
        // f0 が既に先頭なので全体を動かさない
        assert_eq!(names(&app), vec!["f0", "f1", "f2", "f3"]);
    }

    #[test]
    fn duplicate_selected_inserts_copies_directly_after_each_source() {
        let mut app = app_with_folders(3);
        app.selected_items = [0, 2].into_iter().collect();
        app.duplicate_selected();
        assert_eq!(names(&app), vec!["f0", "f0", "f1", "f2", "f2"]);
    }

    #[test]
    fn copy_and_paste_appends_clones_to_the_end() {
        let mut app = app_with_folders(2);
        app.selected_items = [0].into_iter().collect();
        app.copy_selected();
        app.paste_clipboard();
        assert_eq!(names(&app), vec!["f0", "f1", "f0"]);
    }

    #[test]
    fn move_selected_to_menu_transfers_items_to_the_submenu() {
        let mut app = app_with_folders(3);
        app.config.items.push(Item::Submenu {
            name: "Sub".to_string(),
            items: Vec::new(),
            show_branch: false,
        });
        app.selected_items = [0, 1].into_iter().collect();
        app.move_selected_to_menu(&[3]);
        assert_eq!(names(&app), vec!["f2", "Sub"]);
        let Item::Submenu { items, .. } = &app.config.items[1] else {
            panic!("expected submenu");
        };
        assert_eq!(
            items
                .iter()
                .map(|i| i.label().unwrap_or_default().to_string())
                .collect::<Vec<_>>(),
            vec!["f0", "f1"]
        );
    }

    #[test]
    fn move_selected_to_menu_refuses_to_move_into_the_selection_itself() {
        let mut app = app_with_folders(0);
        app.config.items.push(Item::Submenu {
            name: "Sub".to_string(),
            items: Vec::new(),
            show_branch: false,
        });
        app.selected_items = [0].into_iter().collect();
        app.move_selected_to_menu(&[0]);
        // 何も起きない: Sub はまだルートに残ったまま
        assert_eq!(names(&app), vec!["Sub"]);
    }
}
