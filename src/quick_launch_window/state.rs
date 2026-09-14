//! Quick Launch ウィンドウの状態。
//!
//! `mod.rs` が 500 行規約を超えたため分離した。可視性は分離前と同じ
//! (`quick_launch_window` の中だけ) に保つため、`pub(crate)` ではなく
//! `pub(in crate::quick_launch_window)` を使う。クレート全体へ広げると、
//! ウィンドウ実装の内部状態を外のモジュールから触れるようになってしまう。

use std::cell::RefCell;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{HBRUSH, HFONT};

use super::azure_live;
use crate::quick_launch::{Entry, Index};

thread_local! {
    pub(in crate::quick_launch_window) static STATE: RefCell<State> = RefCell::new(State::default());
}

/// リストボックスの 1 行が何を表すか。`results` のインデックスと行番号の
/// ずれを吸収するため、行番号から `results` を直接インデックス参照する
/// 代わりに必ずこの配列 (`State::rows`) を経由する。
#[derive(Debug, Clone, Copy)]
pub(in crate::quick_launch_window) enum RowKind {
    /// `results[usize]` を表示する通常の項目行。選択・実行の対象。
    Item(usize),
    /// 絞り込みなし一覧の区分見出し。選択・実行の対象外。
    Header(&'static str),
    /// 検索中・0 件時の説明文 (`State::empty_message`)。選択・実行の対象外。
    Message,
}

#[derive(Default)]
pub(in crate::quick_launch_window) struct State {
    pub(in crate::quick_launch_window) window: Option<HWND>,
    pub(in crate::quick_launch_window) edit: Option<HWND>,
    pub(in crate::quick_launch_window) list: Option<HWND>,
    pub(in crate::quick_launch_window) owner: Option<HWND>,
    pub(in crate::quick_launch_window) origin: Option<HWND>,
    pub(in crate::quick_launch_window) index: Index,
    /// 拡張から受け取った現在のブラウザタブ。Index を再構築しても失わないよう、
    /// 検索インデックスとは別にメモリ上で保持する。
    pub(in crate::quick_launch_window) browser_tabs:
        Vec<(crate::browser_tabs::Browser, crate::browser_tabs::Tab)>,
    pub(in crate::quick_launch_window) results: Vec<Entry>,
    /// 直前に同期検索した入力。末尾への文字追加だけなら、前回の候補を
    /// 起点に再検索して全索引の走査を避けるために使う。
    pub(in crate::quick_launch_window) previous_query: Option<String>,
    /// リストボックスの行番号ごとの内訳。通常検索時は `Item(0), Item(1), ...`
    /// のフラットな並び (見出しなし)。行番号と `results` の対応を一箇所の
    /// 配列に固定することで、描画・選択移動・実行の各所で見出し行と項目行の
    /// 変換ロジックを重複させない。
    pub(in crate::quick_launch_window) rows: Vec<RowKind>,
    pub(in crate::quick_launch_window) pending: Option<Entry>,
    /// `Ctrl+Alt+Enter` で管理者実行を要求されたときに立てる (FR-9.8.4)。
    /// `Action` の変種を増やすと履歴・バッジ・to_item の各 match に
    /// 昇格の有無が漏れ出すため、実行経路だけに効く旗として `pending`
    /// と対で持つ。
    pub(in crate::quick_launch_window) pending_elevated: bool,
    /// `Ctrl+Shift+Enter` で config への登録を要求された項目。
    /// ウィンドウは閉じずに続けて検索できるようにするため、
    /// `pending` (Enter で実行する項目) とは別に持つ。
    pub(in crate::quick_launch_window) pending_add: Option<Entry>,
    /// `Ctrl+C` でパスをクリップボードへコピーした直後に立てる。
    /// 検索窓のバッジを一時的に `COPIED` へ差し替えるのに使い、
    /// 次のキー入力 (`update_results`) で通常のバッジへ戻る。
    pub(in crate::quick_launch_window) copy_feedback: bool,
    pub(in crate::quick_launch_window) visible_results: usize,
    pub(in crate::quick_launch_window) dpi: u32,
    pub(in crate::quick_launch_window) edit_font: Option<HFONT>,
    pub(in crate::quick_launch_window) name_font: Option<HFONT>,
    pub(in crate::quick_launch_window) detail_font: Option<HFONT>,
    pub(in crate::quick_launch_window) background_brush: Option<HBRUSH>,
    pub(in crate::quick_launch_window) surface_brush: Option<HBRUSH>,
    pub(in crate::quick_launch_window) everything_enabled: bool,
    pub(in crate::quick_launch_window) azure_devops: crate::config::AzureDevOpsSettings,
    /// `f ` プレフィックスの間だけ立てる。プレフィックスを抜けた後に
    /// 遅れて届く Everything の応答を、無関係な検索結果へ混ぜないための
    /// ガード。
    pub(in crate::quick_launch_window) everything_active: bool,
    /// 最後に送った Everything クエリの応答 ID。高速に入力したとき、
    /// 先行クエリの応答が後から届いて現在の候補を上書きするのを防ぐ。
    pub(in crate::quick_launch_window) everything_reply_id: u32,
    /// `f ` モード中に有効な Everything 検索フラグ
    /// (`everything::MATCH_CASE` 等の OR 合成)。モードを抜けても値は保持し、
    /// 次に `f ` へ入ったときも同じ絞り込みを引き継ぐ。
    pub(in crate::quick_launch_window) everything_flags: u32,
    /// `az wit ` 中だけ立てる。古い検索スレッドの結果を捨てるために使う。
    pub(in crate::quick_launch_window) azure_work_items_active: bool,
    pub(in crate::quick_launch_window) azure_work_item_reply_id: u32,
    pub(in crate::quick_launch_window) azure_work_item_query: String,
    /// PR のライブ検索 (`AzureLivePullRequestSearch`) 中だけ立てる。
    /// `azure_work_item_*` と同じ役割だが、Work Item のライブ検索と
    /// reply_id の名前空間を分けるために独立させている。
    pub(in crate::quick_launch_window) azure_pull_requests_live_active: bool,
    pub(in crate::quick_launch_window) azure_pull_request_reply_id: u32,
    /// Pipeline のライブ検索 (`AzureLivePipelineSearch`) 中だけ立てる。
    /// Pipeline は永続キャッシュを持たないので、`az pipeline ` に入る
    /// たびにこの経路を通る (PR/Work Item と違いキャッシュ検索を挟まない)。
    pub(in crate::quick_launch_window) azure_pipelines_live_active: bool,
    pub(in crate::quick_launch_window) azure_pipeline_reply_id: u32,
    pub(in crate::quick_launch_window) azure_live_search_gate: azure_live::LiveSearchGate,
    /// `az <query>` (サブコマンド無し) の `Ctrl+Enter` で 3 種の Live 検索を
    /// 同時に投げたときだけ `Some`。到着順に結果をここへ積み、都度リストを
    /// 組み直す (単独種別の検索は従来どおり `results` を丸ごと置き換える)。
    pub(in crate::quick_launch_window) azure_live_combined: Option<azure_live::CombinedLiveSearch>,
    /// 非同期検索中・0 件時に結果一覧へ出す説明。実行対象にはしない。
    pub(in crate::quick_launch_window) empty_message: Option<String>,
    /// 現在の入力が `b `/`w `/`a `/`f ` のいずれかに入っていれば
    /// そのモード名。検索窓のバッジ表示に使う。
    pub(in crate::quick_launch_window) badge: Option<&'static str>,
    /// 現在の入力で `Ctrl+Enter` の Live 検索が実際に成立するか。
    /// `az project` / `az optimize` のように検索対象を持たないコマンドでは
    /// 立たない。描画のたびに入力を読み直さずに済むよう、バッジと同じく
    /// 入力が変わった時点 (`update_badge`) で確定させる。
    pub(in crate::quick_launch_window) live_search_hint: bool,
    /// モードプレフィックス除去済みの検索語。空なら一覧の候補名を
    /// ハイライトしない (絞り込みなし一覧や Everything / Azure の
    /// 非同期検索など、一致箇所が `name` に対応しない場合)。
    pub(in crate::quick_launch_window) highlight_term: String,
}
