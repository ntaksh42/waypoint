//! 候補リストの構築・差し替え。

use super::layout::{position_window, rows_height};
use super::{Entry, RowKind, STATE};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{LB_ADDSTRING, LB_RESETCONTENT, LB_SETCURSEL};
use windows::core::HSTRING;

/// リストボックスの中身を丸ごと差し替える。通常検索と Everything の
/// 非同期結果受信 (`handle_everything_results`) の双方から使う。
/// `rows` が空でも構わない (見出しも項目もない = 空一覧)。
/// 初期カーソルは、見出し行を飛ばした最初の項目行に置く。
///
/// 合わせてウィンドウの高さを `rows` の実際の行数へ合わせ直す。候補数が
/// 変わるたびに呼ぶことで、候補が少ないときに下部の空きリストボックス分の
/// 余白が残るのを防ぐ。
pub(super) fn populate_list(list: HWND, labels: &[HSTRING], rows: &[RowKind]) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            list,
            LB_RESETCONTENT,
            None,
            None,
        );
        for label in labels {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        if let Some(row) = rows.iter().position(|row| matches!(row, RowKind::Item(_))) {
            let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                LB_SETCURSEL,
                Some(WPARAM(row)),
                None,
            );
        }
    }
    resize_to_rows(rows);
    super::draw_footer::invalidate_footer();
}

/// `populate_list` から呼ぶ、ウィンドウ再配置の実処理。表示中でなければ何もしない。
fn resize_to_rows(rows: &[RowKind]) {
    let (window, dpi, visible_results) = STATE.with(|state| {
        let state = state.borrow();
        (state.window, state.dpi, state.visible_results)
    });
    let Some(window) = window else {
        return;
    };
    let height = rows_height(rows, visible_results);
    position_window(window, height, dpi);
}

/// `results` と区分見出し (`results` 側インデックス昇順の `(挿入位置, ラベル)`) から、
/// リストボックスへ渡す行ラベルと `State::rows` を作る。
/// `section_headers` が空なら見出しなしのフラットな 1:1 対応になる。
pub(super) fn build_rows(
    results: &[Entry],
    section_headers: &[(usize, &'static str)],
) -> (Vec<HSTRING>, Vec<RowKind>) {
    let mut labels = Vec::with_capacity(results.len() + section_headers.len());
    let mut rows = Vec::with_capacity(results.len() + section_headers.len());
    let mut headers = section_headers.iter().peekable();
    for (index, entry) in results.iter().enumerate() {
        while let Some((at, label)) = headers.peek() {
            if *at != index {
                break;
            }
            labels.push(HSTRING::from(*label));
            rows.push(RowKind::Header(label));
            headers.next();
        }
        labels.push(HSTRING::from(format!(
            "{}    {}",
            entry.name,
            super::draw_row::entry_context(entry)
        )));
        rows.push(RowKind::Item(index));
    }
    // 末尾 (results が空、または最後の区分) に付く見出しも取りこぼさない
    for (_, label) in headers {
        labels.push(HSTRING::from(*label));
        rows.push(RowKind::Header(label));
    }
    (labels, rows)
}

/// 結果がある場合も鮮度・範囲・部分失敗を先頭の非選択行に出す。
pub(super) fn with_message(
    mut labels: Vec<HSTRING>,
    mut rows: Vec<RowKind>,
    message: Option<&str>,
) -> (Vec<HSTRING>, Vec<RowKind>) {
    if let Some(message) = message {
        labels.insert(0, HSTRING::from(message));
        rows.insert(0, RowKind::Message);
    }
    (labels, rows)
}

/// 説明用の 1 行を出す。`results` には追加しないため Enter で実行されない。
/// 呼び出し元は `state.rows` もこの内容に合わせて更新すること
/// (古い `RowKind::Item` が残ると、表示上は消えた項目を選択・実行できてしまう)。
pub(super) fn populate_empty_message(list: HWND, message: Option<&str>) {
    let labels = message.map(HSTRING::from).into_iter().collect::<Vec<_>>();
    let rows = vec![RowKind::Message; labels.len()];
    populate_list(list, &labels, &rows);
}
