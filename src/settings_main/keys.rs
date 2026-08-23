//! ダイアログの既定キー処理 (Enter 確定 / Esc 取り消し) とフォーカス制御。

use eframe::egui;

/// `Modifiers` を厳密に見て打鍵を 1 つ取り出す。
///
/// egui の `consume_key` は `matches_logically` で照合するため、指定していない
/// Shift / Alt を無視する。素の `Enter` を待っているつもりが `Shift+Enter` まで
/// 拾ってしまうので、ここでは `matches_exact` で見る。
pub(super) fn consume_key_exact(
    ctx: &egui::Context,
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> bool {
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
pub(super) fn lock_modal_focus<R>(ctx: &egui::Context, window: &Option<egui::InnerResponse<R>>) {
    if let Some(window) = window {
        ctx.memory_mut(|memory| memory.set_modal_layer(window.response.layer_id));
    }
}

/// フォーカス中のウィジェットが `Enter` を自分の起動に使うか。
///
/// egui はフォーカス中のクリック可能なウィジェットを `Enter` で押す。
/// そこへ画面側の既定動作を重ねると、Cancel にフォーカスがある状態の
/// `Enter` が「取り消し」と「確定」の両方を起こしてしまう。
pub(super) fn focus_takes_enter(ctx: &egui::Context) -> bool {
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
pub(super) fn dialog_keys(ctx: &egui::Context, accept: bool) -> (bool, bool) {
    let ok = accept
        && !focus_takes_enter(ctx)
        && consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Enter);
    let cancel = consume_key_exact(ctx, egui::Modifiers::NONE, egui::Key::Escape);
    (ok, cancel)
}
