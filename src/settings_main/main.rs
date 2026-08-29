//! waypoint 設定エディター (FR-6)。常駐部とは別プロセスで必要時のみ起動する。

#![windows_subsystem = "windows"]

mod app;
mod app_edit;
mod azure_draft;
mod drafts;
mod helpers;
mod keys;
mod shortcuts;
#[cfg(test)]
mod tests;
mod trigger_draft;
mod ui_azure;
mod ui_azure_detail;
mod ui_azure_suggest;
mod ui_confirm;
mod ui_editors;
mod ui_import;
mod ui_items;
mod ui_toolbar;
mod ui_tree;
mod ui_trigger;

use eframe::egui;

use app::SettingsApp;
use azure_draft::AzureProjectPicker;
use drafts::{ImportDraft, MoveToMenuDraft, VariablesDraft};
use ui_items::reorder_target;

fn main() -> eframe::Result<()> {
    // 設定画面も GUI サブシステム。panic を握り潰さずログへ残す
    waypoint::panic_log::install();

    let icon = app_icon();
    let options = eframe::NativeOptions {
        // Azure DevOps 設定 (一覧 + 詳細の 2 ペイン) が最も広い領域を要る。
        // egui::Window はホストのビューポートを超えられないため、ここが
        // 狭いとダイアログごと切り詰められて中身が見切れる。
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([820.0, 560.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "Customize - Waypoint",
        options,
        Box::new(move |creation| {
            configure_fonts(&creation.egui_ctx);
            creation.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(SettingsApp::load()))
        }),
    )
}

/// egui の既定フォント (Hack / Ubuntu-Light) は CJK グリフを持たない。
/// 項目名やパスに日本語が入っていると豆腐 (□) になって文字化けして見える
/// ので、Windows 同梱の CJK フォントをフォールバックとして追加する。
/// 先頭に挿す (既定フォントを置き換える) のではなく末尾に足すことで、
/// 英数字は従来どおり既定フォントで描かれ、CJK 文字だけこちらへ落ちる。
/// 候補が 1 つも見つからない環境ではフォント追加をせず既定のまま動かす。
fn configure_fonts(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let candidates = [
        "Fonts\\YuGothM.ttc",  // Yu Gothic Medium (Windows 10/11 既定)
        "Fonts\\meiryo.ttc",   // Meiryo (Vista 以降)
        "Fonts\\msgothic.ttc", // MS Gothic (常に存在する最終フォールバック)
    ];
    let Some(bytes) = candidates
        .into_iter()
        .find_map(|rel| std::fs::read(std::path::Path::new(&windir).join(rel)).ok())
    else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    let name = "system-cjk".to_owned();
    fonts
        .font_data
        .insert(name.clone(), egui::FontData::from_owned(bytes).into());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(name.clone());
    }
    ctx.set_fonts(fonts);
}

/// 項目一覧の列幅。見出しと各行で共有する。
const COL_NAME: f32 = 170.0;
const COL_KIND: f32 = 96.0;
const COL_OPEN: f32 = 82.0;
/// 見出し行のグリップ分の字下げ。
const ROW_INDENT: f32 = 20.0;
/// 一覧末尾に置く受け口の高さ。ここへ落とすと最後尾へ移る。
const TAIL_DROP_HEIGHT: f32 = 24.0;

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../../assets/waypoint.png"))
        .expect("埋め込みアイコンの読み込みに失敗")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}
