//! Quick Launch の再入時の借用に関する回帰テスト。
//!
//! 実際の window_proc は Win32 が必要なため統合テストから直接は叩けない。
//! ここでは同じ構造 (借用中に同期メッセージで再入する) を再現し、
//! 「借用を解放してから Win32 を呼ぶ」形なら panic しないことを固定する。

use std::cell::RefCell;

#[derive(Default)]
struct State {
    results: Vec<String>,
    list: Option<usize>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// WM_ERASEBKGND / WM_DRAWITEM 相当。再入して borrow() する。
fn on_reentrant_message() -> usize {
    STATE.with(|state| state.borrow().results.len())
}

/// SendMessageW 相当。同期的に window_proc を再入させる。
fn send_message_sync() -> usize {
    on_reentrant_message()
}

/// 修正後の update_results と同じ形。
/// 借用内では計算のみ行い、解放してから「Win32」を呼ぶ。
fn update_results_fixed(state: &RefCell<State>) -> usize {
    let (list, labels) = {
        let mut state = state.borrow_mut();
        state.results = vec!["a".to_string(), "b".to_string()];
        (state.list, state.results.clone())
    }; // 借用はここで切れる

    let _ = list;
    // 借用を保持していないので、再入しても borrow() が成功する
    let mut seen = 0;
    for _ in &labels {
        seen = send_message_sync();
    }
    seen
}

#[test]
fn reentrant_borrow_does_not_panic_after_fix() {
    STATE.with(|state| {
        state.borrow_mut().list = Some(1);
        let seen = update_results_fixed(state);
        assert_eq!(seen, 2, "再入先から結果が読めること");
    });
}

/// 借用を保持したまま再入すると panic することを明示しておく。
/// これが修正前の挙動であり、同じ形を再び書かないための記録。
#[test]
fn holding_borrow_across_reentry_panics() {
    let cell = RefCell::new(State::default());
    let held = cell.borrow_mut();
    assert!(
        cell.try_borrow().is_err(),
        "可変借用中の borrow() は失敗する = window_proc 再入で panic する"
    );
    drop(held);
    assert!(cell.try_borrow().is_ok(), "解放後は成功する");
}
