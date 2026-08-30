//! 監視プロジェクト一覧の JSON ファイルへの書き出し・読み込み。

use waypoint::config::AzureDevOpsProject;

use super::AzureProjectPicker;

impl AzureProjectPicker {
    /// 監視プロジェクト一覧を JSON ファイルへ書き出す。PAT は含めない
    /// (`AzureDevOpsProject` に PAT フィールド自体が無い) ので、そのまま
    /// 共有・バックアップしてよい。
    pub(crate) fn export_to_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("azure-devops-projects.json")
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        match encode_projects_json(&self.projects) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => {
                    self.error = None;
                    self.status = Some(format!(
                        "Exported {} project(s) to {}.",
                        self.projects.len(),
                        path.display()
                    ));
                }
                Err(error) => {
                    self.status = None;
                    self.error = Some(format!("Could not write file: {error}"));
                }
            },
            Err(error) => {
                self.status = None;
                self.error = Some(format!("Could not encode projects: {error}"));
            }
        }
    }

    /// JSON ファイルから監視プロジェクト一覧を読み込み、現在の一覧を丸ごと置き換える。
    /// 開いていた詳細パネルは選択解除する (置き換え後は別のエントリを指すため)。
    pub(crate) fn import_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|error| format!("Could not read file: {error}"))
            .and_then(|contents| decode_projects_json(&contents));
        match result {
            Ok(projects) => {
                let count = projects.len();
                self.projects = projects;
                self.selected = None;
                self.error = None;
                self.status = Some(format!(
                    "Imported {count} project(s) from {}.",
                    path.display()
                ));
            }
            Err(error) => {
                self.status = None;
                self.error = Some(error);
            }
        }
    }
}

pub(crate) fn encode_projects_json(projects: &[AzureDevOpsProject]) -> Result<String, String> {
    serde_json::to_string_pretty(projects).map_err(|error| error.to_string())
}

pub(crate) fn decode_projects_json(contents: &str) -> Result<Vec<AzureDevOpsProject>, String> {
    serde_json::from_str(contents).map_err(|error| format!("Could not parse file: {error}"))
}
