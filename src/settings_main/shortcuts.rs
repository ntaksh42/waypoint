//! 画面全体のキーボードショートカット。

use eframe::egui;

use super::app::SettingsApp;
use super::keys::{consume_key_exact, focus_takes_enter};

impl SettingsApp {
    /// Shift+矢印での範囲選択伸縮。egui の標準フォーカス移動 (Tab 系) が
    /// 矢印キーを Shift 修飾ごと横取りする前に、一覧の描画より先に消費する。
    pub(super) fn handle_range_selection_keys(&mut self, ctx: &egui::Context) {
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
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
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
}
