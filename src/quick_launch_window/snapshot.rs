//! 都度取るスナップショット (`k ` / `cs `) の絞り込み表示。

use std::cell::RefCell;

use super::search::{build_rows, populate_list};
use super::{MAX_LIST_RESULTS, State};
use crate::quick_launch::Entry;

/// `k ` (実行中プロセス、FR-9.15.2) / `cs ` (過去セッション、FR-9.15.6) の
/// スナップショットを、残りの文字列で絞り込む。Everything と違い外部 IPC を
/// 伴わないので、非同期にせずキー入力のたびに同期で完結する。
pub(super) fn show_snapshot_results(
    state: &RefCell<State>,
    entries: &[Entry],
    text: &str,
    paths: bool,
) {
    let (list, labels, rows) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        state.empty_message = None;
        state.previous_query = None;
        state.highlight_term = text.to_string();
        state.results =
            crate::quick_launch::search_entries(entries, text, paths, &state.index.ranking)
                .into_iter()
                .take(MAX_LIST_RESULTS)
                .cloned()
                .collect();
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = rows.clone();
        (state.list, labels, rows)
    };
    let Some(list) = list else {
        return;
    };
    populate_list(list, &labels, &rows);
}
