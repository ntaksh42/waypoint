//! Quick Launch 画面の寸法・配色・上限値。
//!
//! `mod.rs` が 500 行規約を超えたため分離した。値そのものの定義だけを置き、
//! 参照側は従来どおり `super::BACKGROUND` のように辿れるよう
//! `mod.rs` 側で再公開する。

use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::WindowsAndMessaging::WM_APP;
use windows::core::{PCWSTR, w};

pub(crate) const EDIT_ID: isize = 1001;
pub(crate) const LIST_ID: isize = 1002;
pub(crate) const WINDOW_WIDTH: i32 = 720;
pub(crate) const PADDING: i32 = 10;
/// 検索窓の高さ。虫眼鏡アイコンと 16px の入力文字を収めるため 34 → 40 へ広げた。
pub(crate) const EDIT_HEIGHT: i32 = 40;
/// 候補行の高さ。名前 15px・詳細 13px の 2 段が重ならない最小値。
/// 旧 42px では名前 (上端 2〜23px) と詳細 (20px〜) の描画範囲が重なっていた。
pub(crate) const ROW_HEIGHT: i32 = 48;
/// セクション見出し行の高さ。通常項目より詰めて、区切りだと分かる程度にする。
pub(crate) const HEADER_HEIGHT: i32 = 32;
/// ウィンドウ下端の操作ヒント帯の高さ。
pub(crate) const FOOTER_HEIGHT: i32 = 32;
/// リストと操作ヒント帯の間の隙間。
pub(crate) const FOOTER_GAP: i32 = 6;
/// モードバッジ ("BOOKMARKS" 等) 用に検索窓の右側へ確保する幅。
pub(crate) const BADGE_WIDTH: i32 = 92;
/// 検索窓の左端に描く虫眼鏡アイコンのために、入力欄を右へずらす幅。
pub(crate) const SEARCH_ICON_WIDTH: i32 = 30;
/// 候補行のアイコン一辺。
pub(crate) const ICON_SIZE: i32 = 26;
/// アイコンの左端 (行の左端からの距離)。
pub(crate) const ICON_LEFT: i32 = 12;
/// アイコンからテキストまでの隙間。
pub(crate) const ICON_TEXT_GAP: i32 = 12;
/// テキストの開始位置 (行の左端からの距離)。
pub(crate) const TEXT_LEFT: i32 = ICON_LEFT + ICON_SIZE + ICON_TEXT_GAP;
/// 行の右端に置くタグ (種別・ブランチ・実行操作) の右余白。
pub(crate) const TAG_RIGHT: i32 = 12;

pub(crate) const BACKGROUND: COLORREF = rgb(17, 17, 17);
pub(crate) const SURFACE: COLORREF = rgb(28, 28, 28);
/// 検索窓の縁取り。塗り (SURFACE) と同系だと輪郭が消えるため、はっきり明るくする。
pub(crate) const SURFACE_BORDER: COLORREF = rgb(74, 74, 74);
/// 検索窓左端の虫眼鏡。
pub(crate) const SEARCH_ICON: COLORREF = rgb(154, 154, 154);
pub(crate) const SURFACE_HOVER: COLORREF = rgb(44, 41, 38);
/// 選択行の塗り。旧実装は枠線 + 左端のアクセントバーで示していたが、
/// 選択位置を一目で追えるよう行全体を青で塗りつぶす方式へ変えた。
/// 背景 (BACKGROUND) とのコントラスト比 3.2:1。
pub(crate) const SELECTED_BG: COLORREF = rgb(48, 104, 154);
pub(crate) const ACCENT: COLORREF = rgb(111, 168, 201);
pub(crate) const TEXT_PRIMARY: COLORREF = rgb(245, 245, 245);
/// 詳細行の breadcrumb (主)。
pub(crate) const TEXT_SECONDARY: COLORREF = rgb(189, 189, 189);
/// 詳細行の path (従) や時計など、一段控えめにする補助テキスト用。
/// 旧値 rgb(205,199,190) は TEXT_SECONDARY より明るく、補足のパスが
/// 主の breadcrumb より目立つ逆転が起きていた。名前 → breadcrumb → path の
/// 順に暗くなるよう引き下げてある (背景とのコントラスト比 5.1:1)。
pub(crate) const TEXT_MUTED: COLORREF = rgb(133, 133, 133);
/// 選択行 (SELECTED_BG の上) の breadcrumb。
pub(crate) const SELECTED_TEXT_SECONDARY: COLORREF = rgb(225, 236, 244);
/// 選択行 (SELECTED_BG の上) の path。SELECTED_BG に対して 4.7:1 を確保する。
pub(crate) const SELECTED_TEXT_MUTED: COLORREF = rgb(218, 231, 241);
/// 選択行の中で検索語に一致した箇所の色。ACCENT のままでは SELECTED_BG に
/// 対して 1.7:1 しかなく読めないため、同系の淡い青にする。
/// 名前 (TEXT_PRIMARY) との差は色だけでは小さいので、下線付き太字
/// (`State::highlight_font`) と組み合わせて区別する。
pub(crate) const SELECTED_HIGHLIGHT: COLORREF = rgb(201, 236, 255);
/// セクション見出しのラベル。
pub(crate) const HEADER_TEXT: COLORREF = rgb(154, 154, 154);
/// セクション見出しのラベル右に引く区切り線。
pub(crate) const HEADER_RULE: COLORREF = rgb(42, 42, 42);
/// 行の右端に出す種別タグの地と文字。
pub(crate) const TAG_BG: COLORREF = rgb(38, 38, 38);
pub(crate) const TAG_TEXT: COLORREF = rgb(181, 181, 181);
/// Git ブランチのタグ (枠線と文字)。
pub(crate) const BRANCH_BORDER: COLORREF = rgb(60, 90, 72);
pub(crate) const BRANCH_TEXT: COLORREF = rgb(143, 199, 162);
/// 選択行の上に出すタグ (ブランチ・実行操作) の枠線。
pub(crate) const SELECTED_TAG_BORDER: COLORREF = rgb(110, 156, 192);
/// 操作ヒント帯の地・上端の区切り線・文字・キー表記の枠線とその文字。
pub(crate) const FOOTER_BG: COLORREF = rgb(24, 24, 24);
pub(crate) const FOOTER_RULE: COLORREF = rgb(38, 38, 38);
pub(crate) const FOOTER_TEXT: COLORREF = rgb(154, 154, 154);
pub(crate) const KEYCAP_BORDER: COLORREF = rgb(62, 62, 62);
pub(crate) const KEYCAP_TEXT: COLORREF = rgb(208, 208, 208);

/// Everything からの検索結果を識別する `WM_COPYDATA` の `dwData` の初期値。
/// クエリごとに増やし、入力前の古い応答を判別できるようにする。
pub(crate) const EVERYTHING_REPLY_ID_START: u32 = WM_APP + 5;
/// Quick Launch が一度に Everything へ要求する最大件数。
/// 全件表示はしない (`visible_results` の上限と同じ枠で足りる)。
pub(crate) const EVERYTHING_MAX_RESULTS: u32 = 24;
/// リストボックスへ流し込む候補の上限。
///
/// ウィンドウの高さは `visible_results` (12〜24) で決まるので、それを超える
/// 行を作っても画面には出ない。にもかかわらず 1 キー入力ごとに
/// `HSTRING` の生成 → `LB_ADDSTRING` の同期送信 → オーナードローの
/// `WM_MEASUREITEM` 再入が候補数だけ走るため、母集団が大きいモードでは
/// そのまま体感のカクつきになる (実測: `az wit` のキャッシュは 300 件規模で、
/// 24 行しか見えないのに毎打鍵で全件ぶんを構築していた)。
/// Everything が `EVERYTHING_MAX_RESULTS` で先に絞っているのと同じ理由・
/// 同じ枠を、ローカル検索の結果にも適用する。
pub(crate) const MAX_LIST_RESULTS: usize = 24;
pub(crate) const CLASS_NAME: PCWSTR = w!("WaypointQuickLaunchWindow");

pub(crate) const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}
