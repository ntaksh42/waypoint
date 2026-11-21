//! `az wit` の属性 (担当・種別・状態) による絞り込み (FR-9.18 属性表)。
//!
//! キャッシュ検索と Live 検索の結果の両方に同じ判定を当てる。Live 検索では
//! API へ条件を渡さず、取得した結果をここで絞る。

use super::AzureMeta;

/// `az wit` の状態属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkItemState {
    #[default]
    All,
    /// 完了していないもの (`AzureMeta::is_closed` でないもの)
    Open,
    /// Closed / Done / Removed
    Closed,
}

/// 種別属性のビット。複数指定はいずれかに一致すればよい。
const BUG: u8 = 1;
const TASK: u8 = 1 << 1;
const STORY: u8 = 1 << 2;
const FEATURE: u8 = 1 << 3;
const EPIC: u8 = 1 << 4;

/// `az wit` の絞り込み条件。`Default` は絞り込み無し。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkItemFilter {
    pub(crate) mine: bool,
    /// 種別のビット集合。0 は種別で絞らない。
    pub(crate) types: u8,
    pub(crate) state: WorkItemState,
}

impl WorkItemFilter {
    /// 属性トークン 1 個を反映する。属性でなければ `false`。`token` は小文字化済み。
    pub(super) fn apply(&mut self, token: &str) -> bool {
        match token {
            "mine" | "me" => self.mine = true,
            "bug" | "bugs" => self.types |= BUG,
            "task" | "tasks" => self.types |= TASK,
            "story" | "stories" => self.types |= STORY,
            "feature" | "features" => self.types |= FEATURE,
            "epic" | "epics" => self.types |= EPIC,
            "open" => self.state = WorkItemState::Open,
            "closed" => self.state = WorkItemState::Closed,
            _ => return false,
        }
        true
    }

    /// 絞り込み条件が 1 つでもあるか。
    pub fn is_filtered(&self) -> bool {
        *self != Self::default()
    }

    /// Work Item の候補が条件を満たすか。メタ情報の無い候補は通さない。
    pub fn matches(&self, meta: Option<&AzureMeta>) -> bool {
        if !self.is_filtered() {
            return true;
        }
        let Some(meta) = meta else {
            return false;
        };
        if self.mine && !meta.is_mine {
            return false;
        }
        if self.types != 0 && self.types & type_bit(meta.work_item_type.as_deref()) == 0 {
            return false;
        }
        match self.state {
            WorkItemState::All => true,
            WorkItemState::Open => !meta.is_closed(),
            WorkItemState::Closed => meta.is_closed(),
        }
    }
}

/// `System.WorkItemType` を種別ビットへ。`story` はプロセステンプレートごとの
/// 要求の種別 (Agile: User Story / Scrum: Product Backlog Item / CMMI: Requirement) をまとめる。
fn type_bit(work_item_type: Option<&str>) -> u8 {
    let Some(name) = work_item_type else {
        return 0;
    };
    [
        ("Bug", BUG),
        ("Task", TASK),
        ("User Story", STORY),
        ("Product Backlog Item", STORY),
        ("Requirement", STORY),
        ("Feature", FEATURE),
        ("Epic", EPIC),
    ]
    .into_iter()
    .find(|(known, _)| name.eq_ignore_ascii_case(known))
    .map_or(0, |(_, bit)| bit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure_devops::Kind;

    fn meta(work_item_type: &str, status: &str, is_mine: bool) -> AzureMeta {
        AzureMeta {
            kind: Kind::WorkItem,
            work_item_type: Some(work_item_type.to_string()),
            work_item_style: None,
            status: status.to_string(),
            is_mine,
            needs_my_review: false,
            is_draft: false,
            ready_to_complete: false,
        }
    }

    fn filter(tokens: &[&str]) -> WorkItemFilter {
        let mut filter = WorkItemFilter::default();
        for token in tokens {
            assert!(filter.apply(token), "{token}");
        }
        filter
    }

    #[test]
    fn unfiltered_accepts_everything() {
        assert!(WorkItemFilter::default().matches(None));
        assert!(WorkItemFilter::default().matches(Some(&meta("Bug", "Done", false))));
    }

    #[test]
    fn types_are_ored_and_story_covers_each_process_template() {
        let bug_or_story = filter(&["bug", "stories"]);
        assert!(bug_or_story.matches(Some(&meta("Bug", "New", false))));
        assert!(bug_or_story.matches(Some(&meta("User Story", "New", false))));
        assert!(bug_or_story.matches(Some(&meta("Product Backlog Item", "New", false))));
        assert!(bug_or_story.matches(Some(&meta("requirement", "New", false))));
        assert!(!bug_or_story.matches(Some(&meta("Task", "New", false))));
        assert!(!bug_or_story.matches(Some(&meta("Custom", "New", false))));
    }

    #[test]
    fn mine_and_state_are_anded_with_types() {
        let my_open_tasks = filter(&["mine", "task", "open"]);
        assert!(my_open_tasks.matches(Some(&meta("Task", "Active", true))));
        assert!(!my_open_tasks.matches(Some(&meta("Task", "Active", false))));
        assert!(!my_open_tasks.matches(Some(&meta("Task", "Done", true))));
        assert!(filter(&["closed"]).matches(Some(&meta("Bug", "Removed", false))));
        assert!(!filter(&["closed"]).matches(Some(&meta("Bug", "Active", false))));
        assert!(!filter(&["mine"]).matches(None));
    }

    #[test]
    fn unknown_tokens_are_not_attributes() {
        let mut filter = WorkItemFilter::default();
        assert!(!filter.apply("live"));
        assert!(!filter.apply("crash"));
        assert!(!filter.is_filtered());
    }
}
