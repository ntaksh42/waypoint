//! 設定画面のキー操作の回帰テスト。
//!
//! 設定画面はバイナリ (`src/settings_main.rs`) なので統合テストから直接は
//! 叩けない。ここでは実装が前提にしている egui 側の性質を固定する。
//! これが崩れると設定画面のキー操作が黙って壊れる。

use eframe::egui;

/// 1 パス描く。`FullOutput` は捨てる前にテクスチャ差分を片付ける必要がある。
fn run(ctx: &egui::Context, input: egui::RawInput, ui: impl FnMut(&mut egui::Ui)) {
    let mut output = ctx.run_ui(input, ui);
    output.textures_delta.clear();
}

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
    egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    }
}

/// `settings_main.rs` の `consume_key_exact` と同じ判定。
/// (バイナリ側の private 関数のため検証用に再実装している)
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

/// UI を組み立てる前に打鍵を消費すると、フォーカス中のウィジェットへ届かない。
/// ショートカット判定を描画後に置いている理由がこれ。
#[test]
fn consuming_before_ui_steals_enter_from_focused_widget() {
    let ctx = egui::Context::default();
    run(&ctx, egui::RawInput::default(), |ui| {
        ui.button("Save").request_focus();
    });

    let mut consumed = false;
    let mut clicked = false;
    run(&ctx, press(egui::Key::Enter, egui::Modifiers::NONE), |ui| {
        consumed = consume_key_exact(ui.ctx(), egui::Modifiers::NONE, egui::Key::Enter);
        clicked = ui.button("Save").clicked();
    });

    assert!(consumed, "先に消費すると打鍵はここで消える");
    assert!(
        !clicked,
        "描画前に消費するとフォーカス中のボタンを Enter で押せない"
    );
}

/// 描画後に見れば、フォーカス中のボタンは Enter で押せる。
/// ただし打鍵はイベントに残るため、横取りするかはフォーカス先を見て決める。
#[test]
fn consuming_after_ui_keeps_enter_for_the_focused_widget() {
    let ctx = egui::Context::default();
    run(&ctx, egui::RawInput::default(), |ui| {
        ui.button("Save").request_focus();
    });

    let mut clicked = false;
    run(&ctx, press(egui::Key::Enter, egui::Modifiers::NONE), |ui| {
        clicked = ui.button("Save").clicked();
    });
    assert!(clicked, "描画後に判定すればボタンが Enter で押せる");

    let still_there = consume_key_exact(&ctx, egui::Modifiers::NONE, egui::Key::Enter);
    assert!(
        still_there,
        "ボタンを押した後も打鍵は残る。フォーカス先を見ないと二重に動く"
    );
}

/// egui の `consume_key` は指定していない Shift / Alt を無視する。
/// `matches_exact` で見れば素の打鍵だけを拾える。
#[test]
fn exact_match_ignores_shift_and_alt_variants() {
    let ctx = egui::Context::default();

    let mut loose = false;
    let mut exact = false;
    run(
        &ctx,
        press(egui::Key::Enter, egui::Modifiers::SHIFT),
        |ui| {
            exact = consume_key_exact(ui.ctx(), egui::Modifiers::NONE, egui::Key::Enter);
            loose = ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        },
    );
    assert!(!exact, "Shift+Enter を素の Enter として拾わないこと");
    assert!(loose, "egui 既定の consume_key は Shift+Enter も拾う");

    let mut exact_delete = false;
    run(&ctx, press(egui::Key::Delete, egui::Modifiers::ALT), |ui| {
        exact_delete = consume_key_exact(ui.ctx(), egui::Modifiers::NONE, egui::Key::Delete);
    });
    assert!(!exact_delete, "Alt+Delete を素の Delete として拾わないこと");
}

/// 一覧の行は矢印キーでフォーカスが移る。ただし `gained_focus()` は立たない。
/// egui が矢印での移動を `end_pass` で差し替えるため、次の描画では
/// 「前フレームから持っていた」扱いになるため。選択の追従は自前で
/// フォーカスの変化を見る必要がある。
#[test]
fn arrow_keys_move_focus_between_rows() {
    /// 3 行を描き、各行の「フォーカスを持つか」「この描画で得たか」を書き出す。
    fn rows(
        ui: &mut egui::Ui,
        focus_first: bool,
        has_focus: &mut [bool; 3],
        gained: &mut [bool; 3],
    ) {
        for index in 0..3 {
            let row = ui.selectable_label(false, format!("row {index}"));
            if focus_first && index == 0 {
                row.request_focus();
            }
            has_focus[index] = row.has_focus();
            gained[index] = row.gained_focus();
        }
    }

    let ctx = egui::Context::default();
    let mut has_focus = [false; 3];
    let mut gained = [false; 3];

    run(&ctx, egui::RawInput::default(), |ui| {
        rows(ui, true, &mut has_focus, &mut gained);
    });
    run(&ctx, egui::RawInput::default(), |ui| {
        rows(ui, false, &mut has_focus, &mut gained);
    });
    assert!(has_focus[0], "1 行目がフォーカスを持つ");

    run(
        &ctx,
        press(egui::Key::ArrowDown, egui::Modifiers::NONE),
        |ui| rows(ui, false, &mut has_focus, &mut gained),
    );
    run(&ctx, egui::RawInput::default(), |ui| {
        rows(ui, false, &mut has_focus, &mut gained);
    });

    assert!(has_focus[1], "ArrowDown で 2 行目へフォーカスが移る");
    assert!(
        !gained[1],
        "矢印で移った行は gained_focus が立たない。選択の追従に使えない"
    );
}

/// `settings_main.rs` の `lock_modal_focus` と同じ判定。
/// (バイナリ側の private 関数のため検証用に再実装している)
fn lock_modal_focus<R>(ctx: &egui::Context, window: &Option<egui::InnerResponse<R>>) {
    if let Some(window) = window {
        ctx.memory_mut(|memory| memory.set_modal_layer(window.response.layer_id));
    }
}

/// egui の `Window` は既定でモーダルではない。開いたまま `Tab` を押すと、
/// フォーカスは画面全体から探され、背後のウィジェットへ抜けてしまう。
///
/// 実機で確認した不具合の再現: 削除確認ダイアログを開いたまま `Tab` を押すと
/// 背後の一覧の行へフォーカスが移り、続けて `Enter` を押すと
/// (行は `Sense::drag()` で `senses_click()` を持たないため、
/// 画面側の「フォーカス中のウィジェットに Enter を譲る」判定に引っかからず)
/// ダイアログの既定動作の方が発火し、確認ボタンを押していないのに確定する。
///
/// `lock_modal_focus` を呼ばない場合にこの漏れが起きることを固定し、
/// 呼べば起きないことも確かめる。
#[test]
fn tab_escapes_window_without_modal_lock_but_not_with_it() {
    // 実際の画面は毎フレーム「背後の一覧」と「ダイアログ」を両方描く
    // (`show_items` の後に `show_confirmations` などを呼ぶ) 。
    // ここでも 1 フレームの中で両方描いて、その競合を再現する。
    struct FrameIds {
        background: egui::Id,
        dialog_layer: egui::LayerId,
    }

    fn frame(ctx: &egui::Context, input: egui::RawInput, lock: bool) -> FrameIds {
        let mut background_id = None;
        let mut window = None;
        let mut output = ctx.run_ui(input, |ui| {
            background_id = Some(ui.button("background row").id);
            window = egui::Window::new("dialog")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    let _ = ui.button("Cancel");
                });
        });
        output.textures_delta.clear();
        if lock {
            lock_modal_focus(ctx, &window);
        }
        FrameIds {
            background: background_id.unwrap(),
            dialog_layer: window.unwrap().response.layer_id,
        }
    }

    /// フォーカス中のウィジェットがダイアログのレイヤーにあるか。
    /// タイトルバーか Cancel ボタンかは問わない。「背後へ抜けていないか」だけを見る
    fn focus_is_in_dialog(ctx: &egui::Context, dialog_layer: egui::LayerId) -> bool {
        let Some(focused) = ctx.memory(|memory| memory.focused()) else {
            return false;
        };
        ctx.read_response(focused)
            .is_some_and(|response| response.layer_id == dialog_layer)
    }

    // モーダル化しない場合: Tab が背後の一覧へ抜ける
    let ctx = egui::Context::default();
    frame(&ctx, egui::RawInput::default(), false);
    let ids = frame(&ctx, egui::RawInput::default(), false);

    frame(&ctx, press(egui::Key::Tab, egui::Modifiers::NONE), false);
    let focused_without_lock = ctx.memory(|memory| memory.focused());
    assert_eq!(
        focused_without_lock,
        Some(ids.background),
        "モーダル化していないと Tab が背後の一覧へ抜ける (現状の不具合と同じ形)"
    );

    // 同じ状況で lock_modal_focus を呼べば、背後は Tab の対象から外れる
    let ctx = egui::Context::default();
    frame(&ctx, egui::RawInput::default(), true);
    let ids = frame(&ctx, egui::RawInput::default(), true);

    frame(&ctx, press(egui::Key::Tab, egui::Modifiers::NONE), true);
    assert!(
        focus_is_in_dialog(&ctx, ids.dialog_layer),
        "lock_modal_focus を呼べば Tab はダイアログの中にとどまる"
    );
}
