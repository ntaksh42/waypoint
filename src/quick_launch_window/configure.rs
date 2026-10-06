//! 設定・動的候補・非同期に構築した索引を Quick Launch の状態へ反映する。
//!
//! いずれも UI スレッドから呼ぶ。索引の差し替えだけ `STATE` の借用内で行い、
//! 表示中なら借用を解放してから結果を描き直す。

use crate::config::Config;
use crate::dynamic::Menus;
use crate::quick_launch::Index;

use super::search::update_results;
use super::{MAX_LIST_RESULTS, STATE};

/// 索引をその場で構築して反映する (同期版)。
///
/// `Index::build` は実測 35〜91ms かかり、起動直後のコールドキャッシュでは
/// さらに伸びる。常駐中の UI スレッドからは呼ばず、`configure_settings` と
/// バックグラウンド構築 + `install_index` の組を使う (`tray::index_build`)。
/// 結果を待ってから終了する `--selftest` 用に残している。
pub fn configure(config: &Config, dynamic: &Menus) {
    configure_settings(config, dynamic);
    install_index(Index::build(config, dynamic), config, dynamic);
}

/// 設定値と config 由来の候補だけを即座に反映する。
///
/// apps / bookmarks / history / azure* はバックグラウンド構築が終わるまで
/// 手元の索引のもの (起動直後なら空) を使い続ける。登録済みフォルダは
/// この時点で検索できるので、構築待ちの間も Quick Launch は使える。
pub fn configure_settings(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            state.index.refresh_config_items(config, dynamic);
            state.previous_query = None;
            state.visible_results = config
                .settings
                .quick_launch
                .visible_results
                .clamp(12, MAX_LIST_RESULTS);
            state.monitor = config.settings.quick_launch.monitor;
            state.everything_enabled = config.settings.quick_launch.include_everything;
            state.azure_devops = config.settings.quick_launch.azure_devops.clone();
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// バックグラウンドで構築した索引へ差し替える。
///
/// 構築に使った config はスレッドへ渡した時点の複製なので、その後の
/// お気に入り登録や存在しない項目の削除を取りこぼさないよう、config 由来の
/// 候補と Recent/Frequent は現在の値で組み直す (0.001ms 程度の軽量処理)。
pub fn install_index(mut index: Index, config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            index.refresh_config_items(config, dynamic);
            let tabs = state.browser_tabs.clone();
            index.set_browser_tabs(&tabs);
            state.index = index;
            state.previous_query = None;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// `configure` の軽量版。Recent/Frequent Folders と開いているウィンドウの
/// 一覧だけを差し替え、apps / bookmarks / history / azure* は保持する
/// (`Index::refresh_dynamic` 参照)。
///
/// `tray::actions::refresh_dynamic` (メニューを閉じるたびに呼ばれる) から
/// 使う。ここで `configure` と同じフル `Index::build` をやり直すと、
/// スタートメニューの COM 解決やブラウザ履歴の SQLite クエリまで
/// 道連れで再実行されて重い (実測)。
pub fn configure_dynamic(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            // タブは `refresh_dynamic` で消えない。別経路 (`replace_browser_tabs`) で更新する
            state.index.refresh_dynamic(config, dynamic);
            // Index の中身 (entries/windows) が変わったので、前回結果への
            // 絞り込み最適化 (`refined_search_term`) をそのまま使い回さない
            state.previous_query = None;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// `configure` の軽量版その 3。config 由来の候補だけを差し替え、
/// apps / bookmarks / history / azure* は保持する
/// (`Index::refresh_config_items` 参照)。
///
/// Quick Launch からのお気に入り登録 (`Ctrl+Shift+Enter`) から使う。
/// 項目が 1 件増えるだけの操作で、変わっていないスタートメニューの
/// 再スキャン (実測で数十 ms) を UI スレッドで走らせない。
///
/// 設定エディターからの保存 (`WM_RELOAD_CONFIG`) はこちらではなく
/// `configure_settings` + バックグラウンド構築を使う。Quick Launch の設定
/// (include_apps など) 自体が変わり得るので、全体を組み直す必要がある。
pub fn configure_config_items(config: &Config, dynamic: &Menus) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            state.index.refresh_config_items(config, dynamic);
            // Index の中身 (entries) が変わったので、前回結果への絞り込み
            // 最適化 (`refined_search_term`) をそのまま使い回さない
            state.previous_query = None;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// `configure` の軽量版その 2。Azure DevOps の候補だけを差し替え、
/// apps / bookmarks / history / Recent/Frequent は保持する。
///
/// バックグラウンド同期の完了通知 (`WM_AZURE_DEVOPS_REFRESHED`) から使う。
/// SQLite の読み取りはバックグラウンドで完了済みなので、UI スレッドでは
/// メモリ上の候補を検索索引へ適用するだけにする。
pub(crate) fn configure_azure(config: &Config, groups: crate::azure_devops::CachedCandidateGroups) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            state
                .index
                .refresh_azure_candidates(&config.settings.quick_launch, groups);
            // Index の中身 (azure*) が変わったので、前回結果への絞り込み
            // 最適化 (`refined_search_term`) をそのまま使い回さない
            state.previous_query = None;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}

/// Native Messaging host が届けた 1 ブラウザ分のタブ一覧を入れ替える。
///
/// タブの変更通知時だけ呼ばれる。Quick Launch が表示中なら、現在の `t ` 検索結果も
/// 即座に差し替えるが、ブラウザへ同期問い合わせは行わない。
pub fn replace_browser_tabs(
    browser: crate::browser_tabs::Browser,
    tabs: Vec<crate::browser_tabs::Tab>,
) {
    STATE.with(|state| {
        let (has_window, edit) = {
            let mut state = state.borrow_mut();
            // 拡張はタブの読み込み中など内容が変わらない通知でも全タブを送る。
            // 同じ一覧なら索引の再構築も再描画もしない
            let unchanged = {
                let mut current = state
                    .browser_tabs
                    .iter()
                    .filter(|(source, _)| *source == browser)
                    .map(|(_, tab)| tab);
                let mut incoming = tabs.iter();
                loop {
                    match (current.next(), incoming.next()) {
                        (None, None) => break true,
                        (Some(old), Some(new)) if old == new => {}
                        _ => break false,
                    }
                }
            };
            if unchanged {
                return;
            }
            state.browser_tabs.retain(|(source, _)| *source != browser);
            state
                .browser_tabs
                .extend(tabs.into_iter().map(|tab| (browser, tab)));
            let state = &mut *state;
            state.index.set_browser_tabs(&state.browser_tabs);
            state.previous_query = None;
            (state.window.is_some(), state.edit)
        };
        // タブを検索するのは `t ` のときだけなので、それ以外の表示は触らない
        let in_tabs_mode = has_window
            && edit.is_some_and(|edit| {
                super::input::read_text(edit).starts_with(crate::quick_launch::TABS_PREFIX)
            });
        if in_tabs_mode {
            update_results(state);
        }
    });
}
