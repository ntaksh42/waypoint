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
pub(crate) const EDIT_HEIGHT: i32 = 34;
pub(crate) const ROW_HEIGHT: i32 = 42;
/// セクション見出し行の高さ。通常項目より詰めて、区切りだと分かる程度にする。
pub(crate) const HEADER_HEIGHT: i32 = 26;
/// モードバッジ ("BOOKMARKS" 等) 用に検索窓の右側へ確保する幅。
pub(crate) const BADGE_WIDTH: i32 = 92;
/// 候補行のアイコン一辺。行の左端からの余白と種別バッジの半径もこれを基準に決める。
pub(crate) const ICON_SIZE: i32 = 26;
/// アイコンの左端 (行の左端からの距離)。
pub(crate) const ICON_LEFT: i32 = 8;
/// アイコンからテキストまでの隙間。
pub(crate) const ICON_TEXT_GAP: i32 = 10;
/// テキストの開始位置 (行の左端からの距離)。
pub(crate) const TEXT_LEFT: i32 = ICON_LEFT + ICON_SIZE + ICON_TEXT_GAP;

pub(crate) const BACKGROUND: COLORREF = rgb(13, 13, 13);
pub(crate) const SURFACE: COLORREF = rgb(32, 30, 28);
pub(crate) const SURFACE_HOVER: COLORREF = rgb(44, 41, 38);
/// 選択行カード専用の背景色。旧実装は `SURFACE_HOVER` を流用していたが、
/// 背景 (BACKGROUND) とのコントラスト比が 1.34:1 しかなく、リスト内を
/// キーボードで移動しても選択位置がほぼ同化して見えなかった (実測)。
/// ACCENT と同系の寒色へ寄せつつ明度を上げ、2.7:1 まで引き上げてある。
pub(crate) const SELECTED_BG: COLORREF = rgb(70, 90, 106);
/// 選択カードの枠線。`SELECTED_BG` 自体が十分明るくなったため、枠は
/// 主張しすぎない程度にアクセントへ寄せる。
pub(crate) const SELECTED_BORDER: COLORREF = rgb(140, 186, 214);
pub(crate) const ACCENT: COLORREF = rgb(111, 168, 201);
pub(crate) const TEXT_PRIMARY: COLORREF = rgb(245, 245, 245);
pub(crate) const TEXT_SECONDARY: COLORREF = rgb(190, 190, 190);
/// detail 行の path (secondary) やセクション見出しなど、一段控えめにする
/// 補助テキスト用。旧 rgb(117,112,106) は選択行の背景 (SURFACE_HOVER) に対する
/// コントラスト比が約2.95:1しかなく視認性が低かった。rgb(148,142,134)・
/// rgb(180,173,163) と段階的に上げてきたが、実機表示でなお他のテキストより
/// 薄く感じるという指摘が続いたため、さらに明度を上げてある。
pub(crate) const TEXT_MUTED: COLORREF = rgb(205, 199, 190);

/// Everything からの検索結果を識別する `WM_COPYDATA` の `dwData` の初期値。
/// クエリごとに増やし、入力前の古い応答を判別できるようにする。
pub(crate) const EVERYTHING_REPLY_ID_START: u32 = WM_APP + 5;
/// Quick Launch が一度に Everything へ要求する最大件数。
/// 全件表示はしない (`visible_results` の上限と同じ枠で足りる)。
pub(crate) const EVERYTHING_MAX_RESULTS: u32 = 24;
/// Outlook の CLI 検索を入力ごとに起動しないためのデバウンス用タイマー。
pub(crate) const OUTLOOK_TIMER_ID: usize = 1;
pub(crate) const OUTLOOK_SEARCH_DELAY_MS: u32 = 250;
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
