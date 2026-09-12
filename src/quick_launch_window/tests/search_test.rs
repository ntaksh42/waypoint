use super::super::search::{
    accepts_azure_work_item_reply, accepts_everything_reply, build_rows,
    invalidate_azure_live_searches, next_everything_reply_id, refinable_search_term,
    start_azure_work_item_query,
};
use super::super::{RowKind, STATE, State};
use crate::config::OpenMode;
use crate::quick_launch::{Action, Entry};
use std::time::{Duration, Instant};

fn folder_entry(name: &str) -> Entry {
    Entry {
        name: name.to_string(),
        breadcrumb: String::new(),
        path: format!(r"C:\{name}"),
        action: Action::OpenFolder(OpenMode::NewWindow),
        branch: None,
    }
}

#[test]
fn stale_everything_reply_is_rejected_after_a_new_query() {
    let first = next_everything_reply_id(0);
    let second = next_everything_reply_id(first);

    assert!(!accepts_everything_reply(true, second, first));
    assert!(accepts_everything_reply(true, second, second));
    assert!(!accepts_everything_reply(false, second, second));
}

#[test]
fn stale_azure_work_item_request_is_rejected_after_more_typing() {
    assert!(!accepts_azure_work_item_reply(true, 8, 7));
    assert!(!accepts_azure_work_item_reply(false, 8, 8));
    assert!(accepts_azure_work_item_reply(true, 8, 8));
}

#[test]
fn editing_a_query_invalidates_all_pending_live_replies() {
    STATE.with(|state| {
        *state.borrow_mut() = State::default();
        {
            let mut state = state.borrow_mut();
            state.azure_work_items_active = true;
            state.azure_work_item_reply_id = 7;
            state.azure_pull_requests_live_active = true;
            state.azure_pipelines_live_active = true;
        }

        invalidate_azure_live_searches(&mut state.borrow_mut());
        start_azure_work_item_query(state, "new query");

        let state = state.borrow();
        assert!(!accepts_azure_work_item_reply(
            state.azure_work_items_active,
            state.azure_work_item_reply_id,
            7,
        ));
        assert!(!state.azure_pull_requests_live_active);
        assert!(!state.azure_pipelines_live_active);
    });
}

#[test]
fn refined_search_only_reuses_candidates_for_a_narrower_local_query() {
    assert_eq!(refinable_search_term(Some("way"), "wayp", 1), Some("wayp"));
    assert_eq!(
        refinable_search_term(Some("b git"), "b gith", 1),
        Some("gith")
    );
    assert_eq!(refinable_search_term(Some("b git"), "h git", 1), None);
    assert_eq!(refinable_search_term(Some("way"), "way", 1), None);
    assert_eq!(refinable_search_term(Some(""), "way", 1), None);
    assert_eq!(refinable_search_term(None, "way", 1), None);
    assert_eq!(refinable_search_term(Some("f read"), "f readm", 1), None);
    assert_eq!(refinable_search_term(Some("az pr"), "az pra", 1), None);
    // 通常検索中に `ps ` を打ち足してターミナル検索モードへ入った直後は、
    // 通常検索の結果 (フォルダ以外を含む) を絞り込むのではなく、
    // Index::search の `ps ` 専用索引を新たに引き直す必要がある。
    // `local_search_scope` が "ps " を認識せず両方 "normal" 扱いになると、
    // ここが誤って Some を返し、フォルダ限定の絞り込みが効かなくなる
    // (実機で `ps` から `ps waypoint` と打った際に再現)。
    assert_eq!(refinable_search_term(Some("ps"), "ps waypoint", 1), None);
    assert_eq!(
        refinable_search_term(Some("ps way"), "ps wayp", 1),
        Some("wayp")
    );
}

/// 前回の結果が表示上限 (`MAX_LIST_RESULTS`) ちょうどで切り詰められていた
/// 場合、絞り込みの母集団を前回の結果だけに限定すると、切り詰めで落ちた
/// 候補 (今回の絞り込みでは本来ヒットするはずのもの) を拾えなくなる。
/// この回だけは最適化を使わず、全候補への再検索に倒すべき。
#[test]
fn refined_search_is_skipped_when_previous_results_were_truncated() {
    assert_eq!(
        refinable_search_term(Some("way"), "wayp", super::super::MAX_LIST_RESULTS),
        None
    );
    // 上限未満なら、切り詰めが起きていないので最適化を使ってよい
    assert_eq!(
        refinable_search_term(Some("way"), "wayp", super::super::MAX_LIST_RESULTS - 1),
        Some("wayp")
    );
}

/// リストへ流し込む上限は、ウィンドウに実際に映る行数 (`visible_results`
/// の上限 24) 以上あれば足り、それを超えて作った行は画面に出ないまま
/// 1 打鍵ごとの `LB_ADDSTRING` と `WM_MEASUREITEM` を増やすだけになる
/// (`az wit` のキャッシュ 300 件規模でカクつきとして表面化した)。
/// 両者がずれると無駄が再発するので、ここで結び付けておく。
#[test]
fn list_result_cap_matches_the_largest_visible_row_count() {
    assert_eq!(super::super::MAX_LIST_RESULTS, 24);
    // Everything も同じ枠で先に絞っている (要求段階での上限)
    assert_eq!(
        super::super::EVERYTHING_MAX_RESULTS as usize,
        super::super::MAX_LIST_RESULTS
    );
}

#[test]
fn build_rows_without_headers_is_a_flat_one_to_one_mapping() {
    let results = vec![folder_entry("a"), folder_entry("b")];
    let (labels, rows) = build_rows(&results, &[]);
    assert_eq!(labels.len(), 2);
    assert!(matches!(
        rows.as_slice(),
        [RowKind::Item(0), RowKind::Item(1)]
    ));
}

#[test]
fn build_rows_inserts_a_header_row_before_each_section_start() {
    let results = vec![folder_entry("a"), folder_entry("b"), folder_entry("c")];
    // "Folders" は results[0] の直前、"Apps" は results[2] の直前に挿入される想定
    let section_headers = [(0, "Folders"), (2, "Apps")];
    let (labels, rows) = build_rows(&results, &section_headers);
    assert_eq!(labels.len(), 5);
    assert!(matches!(
        rows.as_slice(),
        [
            RowKind::Header("Folders"),
            RowKind::Item(0),
            RowKind::Item(1),
            RowKind::Header("Apps"),
            RowKind::Item(2),
        ]
    ));
}

#[test]
fn live_search_gate_blocks_overlap_and_recent_duplicate_but_allows_other_query() {
    use super::super::search::LiveSearchKind::WorkItems;
    let now = Instant::now();
    let mut gate = super::super::search::LiveSearchGate::default();

    assert!(gate.try_start(WorkItems, "wit:one", now));
    assert!(!gate.try_start(WorkItems, "wit:two", now));
    gate.finish(WorkItems, now + Duration::from_secs(10));
    assert!(!gate.try_start(WorkItems, "WIT:ONE", now + Duration::from_secs(11)));
    assert!(gate.try_start(WorkItems, "wit:two", now + Duration::from_secs(11)));
    gate.finish(WorkItems, now + Duration::from_secs(11));
    assert!(gate.try_start(WorkItems, "wit:one", now + Duration::from_secs(12)));
}

/// `az <query>` は 3 種を同時に投げるので、gate は種別ごとに独立している
/// 必要がある。1 本の gate を共有していたとき、2 本目以降が自分自身に
/// 弾かれて Live 検索が 1 種しか走らなかった。
#[test]
fn live_search_gate_allows_all_kinds_at_once() {
    use super::super::search::LiveSearchKind::{Pipelines, PullRequests, WorkItems};
    let now = Instant::now();
    let mut gate = super::super::search::LiveSearchGate::default();

    assert!(gate.try_start(PullRequests, "pr:x", now));
    assert!(gate.try_start(WorkItems, "wit:x", now));
    assert!(gate.try_start(Pipelines, "pipeline:x", now));
}

/// 集約 Live 検索 (`az <query>`) は、全種そろう前でも届いた分を表示し、
/// 最後の応答で確定する。PR → Work Item → Pipeline の順に並べる。
#[test]
fn combined_live_search_merges_results_as_they_arrive() {
    use super::super::search::{CombinedLiveSearch, LiveSearchKind};
    use crate::quick_launch::{Action, Entry};

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            breadcrumb: String::new(),
            path: String::new(),
            action: Action::OpenUrl(String::new()),
            branch: None,
        }
    }

    let mut combined = CombinedLiveSearch::new(3);

    // 1 種目は 0 件でも、残りを待っている間は「検索中」のまま
    let (merged, message) = combined.absorb(LiveSearchKind::WorkItems, Vec::new(), None);
    assert!(merged.is_empty());
    assert_eq!(message.as_deref(), Some("Searching Azure DevOps…"));

    // 2 種目が返れば、残りを待たずにその分を表示する
    let (merged, message) =
        combined.absorb(LiveSearchKind::PullRequests, vec![entry("pr-1")], None);
    assert_eq!(
        merged
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pr-1"]
    );
    assert_eq!(message, None);

    // PR → Work Item → Pipeline の順で並ぶ
    let (merged, message) = combined.absorb(LiveSearchKind::Pipelines, vec![entry("pipe-1")], None);
    assert_eq!(
        merged
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pr-1", "pipe-1"]
    );
    assert_eq!(message, None);
}

/// 全種が 0 件だったときだけ「見つからない」を出す。
#[test]
fn combined_live_search_reports_empty_only_when_all_kinds_finish() {
    use super::super::search::{CombinedLiveSearch, LiveSearchKind};

    let mut combined = CombinedLiveSearch::new(2);
    let (_, message) = combined.absorb(LiveSearchKind::PullRequests, Vec::new(), None);
    assert_eq!(message.as_deref(), Some("Searching Azure DevOps…"));
    let (merged, message) = combined.absorb(LiveSearchKind::WorkItems, Vec::new(), None);
    assert!(merged.is_empty());
    assert_eq!(message.as_deref(), Some("No Azure DevOps results."));
}
