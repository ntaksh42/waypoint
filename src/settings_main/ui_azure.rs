//! Azure DevOps 設定画面の描画 (ウィンドウ骨格・接続行・プロジェクト一覧)。
//!
//! 有効化トグルと監視プロジェクトの選択・詳細編集を 1 つのウィンドウに
//! まとめる。上部に有効化チェックボックスとキャッシュ鮮度、その下に
//! 左は検索・選択可能なプロジェクト一覧、右は選択中プロジェクトの詳細編集
//! (エイリアス・優先度・同期スコープ・興味のある Area Path) を並べる。
//! 大規模な組織でも一覧はフィルタで絞り込め、詳細は 1 プロジェクトずつ
//! 個別に設定できる。OK/Cancel は 1 組だけで、有効化トグルとプロジェクト
//! 選択の両方を一度にコミットする (以前は 2 段のダイアログにそれぞれ
//! 独立した OK/Apply があり、片方だけ保存し忘れる罠があった)。
//!
//! 詳細ペインと優先度提案モーダルは `ui_azure_detail` にある。

use eframe::egui;
use waypoint::config::AzureDevOpsSettings;

use super::app::SettingsApp;
use super::azure_draft::AzureProjectPicker;
use super::keys::{dialog_keys, lock_modal_focus};
use super::ui_azure_detail::show_detail_panel;
use super::ui_azure_suggest::show_priority_suggestion_modal;

/// 左ペイン (一覧) が本文幅に占める割合。残りが右ペイン (詳細) になる。
const LIST_PANE_FRACTION: f32 = 0.38;
/// 左ペインの下限幅。これを割ると、プロジェクト名とエイリアスが
/// 横に潰れて読めなくなる。
const LIST_PANE_MIN_WIDTH: f32 = 220.0;
/// 本文 (一覧 + 詳細) に最低限確保する高さ。
const BODY_MIN_HEIGHT: f32 = 200.0;

impl SettingsApp {
    pub(super) fn show_azure_project_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = self.azure_project_picker.as_mut() else {
            return;
        };
        picker.poll_load();
        // 受信スレッドは egui のイベントループを起こせないため、取得中だけ再描画する。
        // **取得中を表すフラグを 1 つでも落とすと、その取得結果は永久に
        // 反映されない** — poll は描画のたびにしか走らないので、再描画が
        // 予約されないと egui はアイドルのままになる。提案モーダルの
        // Area ツリー (`priority_suggestion_area_loading`) がこれで、
        // 展開しても空のままだった。
        if picker.is_loading_anything() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut apply = false;
        let mut cancel = false;
        // ホストのビューポートを超えるサイズを要求すると、ウィンドウごと
        // 切り詰められて中身が見切れる。実寸から上限を決めて、その中で
        // 収まる既定サイズにする。
        let (host_width, host_height) = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .map_or((1100.0, 760.0), |rect| (rect.width(), rect.height()))
        });
        let max_width = (host_width - 24.0).max(560.0);
        let max_height = (host_height - 24.0).max(400.0);
        let window = egui::Window::new("Azure DevOps")
            .collapsible(false)
            .resizable(true)
            .default_size([max_width.min(1000.0), max_height.min(860.0)])
            .min_size([max_width.min(560.0), max_height.min(400.0)])
            .max_size([max_width, max_height])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // フッター (Export/Import と OK/Cancel) をパネルで先に確保する。
                // 高さを定数で見積もって本文から引く方式だと、ステータス行や
                // エラー行が出た分だけ OK/Cancel がウィンドウ下端の外へ押し
                // 出され、確定できなくなっていた。
                egui::Panel::bottom("azure_footer")
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        top: 6,
                        ..Default::default()
                    }))
                    .show(ui, |ui| {
                        show_footer(ui, picker, &mut apply, &mut cancel);
                    });
                egui::Panel::top("azure_header")
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        bottom: 6,
                        ..Default::default()
                    }))
                    .show(ui, |ui| {
                        show_header(ui, picker);
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        show_body(ui, picker);
                    });
            });
        lock_modal_focus(ctx, &window);

        let (_, dismiss) = dialog_keys(ctx, false);
        cancel |= dismiss;
        if apply {
            picker.commit_text_fields();
            self.config.settings.quick_launch.azure_devops = AzureDevOpsSettings {
                enabled: picker.enabled,
                projects: picker.projects.clone(),
            };
            self.dirty = true;
            self.status = None;
            if self.save() {
                self.azure_project_picker = None;
            }
        } else if cancel {
            self.azure_project_picker = None;
        }

        // Azure DevOps ウィンドウの後に描画することで、モーダルを手前に出す
        // (egui は同じ CENTER_CENTER アンカーだと後から show したウィンドウが
        // 前面に来るため、先に呼ぶと Azure DevOps の後ろに隠れてしまっていた)。
        if let Some(picker) = self.azure_project_picker.as_mut() {
            show_priority_suggestion_modal(ctx, picker);
        }
    }
}

/// 上部: 有効化トグル・キャッシュ鮮度・接続設定・優先度提案。
///
/// 接続設定 (Organization / PAT) は一度入れれば触らないので折りたたむ。
/// 常時展開しておくと本文の高さを 100px 近く食い、狭いウィンドウでは
/// 一覧と詳細が数行しか見えなくなっていた。
fn show_header(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(
            &mut picker.enabled,
            "Enable Azure DevOps search (type \"az \" to search)",
        );
        ui.separator();
        let azure_status = waypoint::azure_devops::cache_status(&AzureDevOpsSettings {
            enabled: picker.enabled,
            projects: picker.projects.clone(),
        });
        ui.weak(waypoint::azure_devops::cache_status_label(&azure_status));
        if let Some(error) = azure_status.last_error {
            ui.weak("·");
            ui.colored_label(
                egui::Color32::from_rgb(220, 140, 90),
                truncate_for_row(&error, 90),
            )
            .on_hover_text(error);
        }
    });

    egui::CollapsingHeader::new("Connection")
        .id_salt("azure_connection")
        // 未接続 (組織が空 / プロジェクト未取得) のときだけ最初から開く。
        // 設定済みなら畳んだまま出して、本文へ高さを譲る。
        .default_open(picker.organization.trim().is_empty() || picker.available_projects.is_empty())
        .show(ui, |ui| show_connection_row(ui, picker));

    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !picker.priority_suggestion_loading && !picker.projects.is_empty(),
                egui::Button::new("Suggest priorities from recent activity..."),
            )
            .on_hover_text(
                "Rank watched projects by your assignments and @mentions \
                 in the last 90 days",
            )
            .clicked()
        {
            picker.start_priority_suggestions();
        }
        if picker.priority_suggestion_loading {
            ui.spinner();
            ui.label("Scanning recent activity...");
        }
    });
    ui.separator();
}

/// 本文: 左に一覧、右に詳細。左右とも親の高さいっぱいに広げ、
/// はみ出す分はそれぞれのペイン内でスクロールさせる。
fn show_body(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    let body_height = ui.available_height().max(BODY_MIN_HEIGHT);
    let total_width = ui.available_width();
    // セパレータと左右の余白ぶんを引いてから配分する。引かずに割ると、
    // 右ペインが親の幅を数 px 超えて末尾の列が切れる。
    let usable_width = (total_width - ui.spacing().item_spacing.x * 2.0 - 8.0).max(120.0);
    let list_width = (usable_width * LIST_PANE_FRACTION)
        .max(LIST_PANE_MIN_WIDTH.min(usable_width * 0.5))
        .min(usable_width - 160.0_f32.min(usable_width * 0.5));

    ui.horizontal_top(|ui| {
        ui.set_min_height(body_height);
        ui.allocate_ui_with_layout(
            egui::vec2(list_width, body_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(list_width);
                show_project_list(ui, picker);
            },
        );
        ui.separator();
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), body_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("azure_detail_panel")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        show_detail_panel(ui, picker);
                    });
            },
        );
    });
}

/// 下部: 入出力と OK/Cancel。ステータス・エラーもここに置き、
/// 出ても本文を押し縮めるだけでボタンは動かないようにする。
fn show_footer(
    ui: &mut egui::Ui,
    picker: &mut AzureProjectPicker,
    apply: &mut bool,
    cancel: &mut bool,
) {
    ui.separator();
    if let Some(status) = &picker.status {
        ui.colored_label(egui::Color32::LIGHT_GREEN, truncate_for_row(status, 120))
            .on_hover_text(status);
    }
    if let Some(error) = &picker.error {
        ui.colored_label(egui::Color32::RED, truncate_for_row(error, 120))
            .on_hover_text(error);
    }
    ui.horizontal(|ui| {
        // 右詰めを先に置く。左詰めを先に流すと、可変長のボタン群が
        // 幅を食い切ったときに OK/Cancel が行外へこぼれる。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            *cancel = ui.button("Cancel").clicked();
            *apply = ui.button("OK").clicked();
            ui.separator();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if ui
                    .button("Export...")
                    .on_hover_text("Save the watched project list to a JSON file")
                    .clicked()
                {
                    picker.export_to_file();
                }
                if ui
                    .button("Import...")
                    .on_hover_text(
                        "Replace the watched project list with one loaded from a JSON file",
                    )
                    .clicked()
                {
                    picker.import_from_file();
                }
            });
        });
    });
}

fn show_connection_row(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    // 入力欄は親の幅に追従させる。固定幅だと、狭いウィンドウで
    // ボタン列ごと右へはみ出して押せなくなる。
    let field_width = (ui.available_width() * 0.28).clamp(120.0, 240.0);
    ui.horizontal_wrapped(|ui| {
        ui.label("Organization");
        ui.add(egui::TextEdit::singleline(&mut picker.organization).desired_width(field_width));
        ui.label("PAT");
        ui.add(
            egui::TextEdit::singleline(&mut picker.pat)
                .password(true)
                .desired_width(field_width),
        );
        if ui.button("Save PAT and load projects").clicked() {
            match waypoint::azure_devops::save_pat(&picker.organization, &picker.pat) {
                Ok(()) => {
                    picker.pat.clear();
                    picker.start_load();
                }
                Err(error) => {
                    picker.status = None;
                    picker.error = Some(error);
                }
            }
        }
        if ui
            .add_enabled(!picker.loading, egui::Button::new("Load projects"))
            .clicked()
        {
            picker.start_load();
        }
        if ui.button("Delete PAT").clicked() {
            match waypoint::azure_devops::delete_pat(&picker.organization) {
                Ok(()) => {
                    picker.error = None;
                    picker.status =
                        Some("PAT removed from Windows Credential Manager.".to_string());
                }
                Err(error) => {
                    picker.status = None;
                    picker.error = Some(error);
                }
            }
        }
        if picker.loading {
            ui.spinner();
            ui.label("Loading...");
        }
    });
    ui.weak("Required PAT scopes: Code (Read), Build (Read), Work Items (Read), Project and Team (Read).");
}

/// 左ペイン: フィルタ・一括操作・チェックボックス一覧。
/// 一覧は親の高さの残り全部を使う (以前は `available_height` から
/// `min_rect().height()` を引いていたが、`min_rect` は Ui の原点からの
/// 矩形でヘッダの消費高ではないため二重に引かれ、一覧が不必要に縮んで
/// いた)。
fn show_project_list(ui: &mut egui::Ui, picker: &mut AzureProjectPicker) {
    ui.add(
        egui::TextEdit::singleline(&mut picker.filter)
            .hint_text("Filter by project name or alias")
            .desired_width(ui.available_width()),
    );
    ui.checkbox(&mut picker.show_selected_only, "Selected only");

    let organization = picker.loaded_organization.clone();
    let aliases_by_project: std::collections::HashMap<String, Vec<String>> = picker
        .projects
        .iter()
        .filter(|entry| entry.organization.eq_ignore_ascii_case(&organization))
        .map(|entry| (entry.project.to_lowercase(), entry.aliases.clone()))
        .collect();
    let aliases_of = |project: &str| -> Vec<String> {
        aliases_by_project
            .get(&project.to_lowercase())
            .cloned()
            .unwrap_or_default()
    };

    let filter = picker.filter.to_lowercase();
    let mut filtered: Vec<_> = picker
        .available_projects
        .iter()
        .filter(|project| picker.is_selected(&organization, project) || !picker.show_selected_only)
        .filter(|project| {
            filter.is_empty()
                || project.to_lowercase().contains(&filter)
                || aliases_of(project)
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(&filter))
        })
        .cloned()
        .collect();
    // 選択済みを先頭に集め、大量の候補の中でも今の選択状態がすぐ見える。
    filtered.sort_by(|a, b| {
        let a_selected = picker.is_selected(&organization, a);
        let b_selected = picker.is_selected(&organization, b);
        b_selected
            .cmp(&a_selected)
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });

    ui.weak(format!(
        "{} of {} shown; {} selected",
        filtered.len(),
        picker.available_projects.len(),
        picker
            .projects
            .iter()
            .filter(|entry| entry.organization.eq_ignore_ascii_case(&organization))
            .count()
    ));
    ui.horizontal_wrapped(|ui| {
        if ui.button("Select shown").clicked() {
            for project in &filtered {
                picker.set_selected(project, true);
            }
        }
        if ui.button("Clear shown").clicked() {
            for project in &filtered {
                picker.set_selected(project, false);
            }
        }
        if ui.button("Clear all").clicked() {
            for project in picker.available_projects.clone() {
                picker.set_selected(&project, false);
            }
        }
    });
    ui.add_space(2.0);

    egui::ScrollArea::vertical()
        .id_salt("azure_project_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if filtered.is_empty() {
                ui.weak(if picker.available_projects.is_empty() {
                    "No projects loaded. Open Connection above and load them."
                } else {
                    "No projects match the filter."
                });
            }
            for project in &filtered {
                let mut checked = picker.is_selected(&organization, project);
                let is_open = picker.selected.as_ref().is_some_and(|(org, proj)| {
                    org.eq_ignore_ascii_case(&organization) && proj == project
                });
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut checked, "").changed() {
                        picker.set_selected(project, checked);
                    }
                    let aliases = aliases_of(project);
                    // 名前とエイリアスを 1 つのラベルにまとめる。別ウィジェット
                    // として横に並べると、長い名前のときにエイリアスがペイン外へ
                    // 出て切れていた。
                    let label_text = if aliases.is_empty() {
                        project.clone()
                    } else {
                        format!("{project}  ({})", aliases.join(", "))
                    };
                    let label = ui.add(
                        egui::Button::selectable(is_open, truncate_for_row(&label_text, 48))
                            .truncate(),
                    );
                    let label = if label_text.chars().count() > 48 {
                        label.on_hover_text(&label_text)
                    } else {
                        label
                    };
                    if label.clicked() && checked {
                        picker.open_detail(&organization, project);
                    }
                });
            }
        });
}

/// 1 行に収まらない長さのテキストを省略する。egui の `truncate()` は
/// ウィジェット幅までしか効かず、`weak`/`colored_label` のような
/// 折り返すラベルには使えないため、文字数で先に切っておく。
pub(super) fn truncate_for_row(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}\u{2026}")
}
