//! Azure DevOps 候補の種別・状態。
//!
//! 行頭アイコン (`quick_launch_window::badge`) と `az` 検索の順位付け
//! (`azure_search`) が参照する。以前はアイコンの種別を URL の文字列から
//! 推定していたが、Work Item Type や PR の状態は URL に現れないため、
//! 候補を作る時点の情報をそのまま持たせる。

use crate::azure_devops::{Candidate, Kind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureMeta {
    pub kind: Kind,
    /// Work Item の `System.WorkItemType` (Bug / Task / User Story 等)。
    pub work_item_type: Option<String>,
    /// PR の状態 (active / completed / abandoned)、Work Item の State、
    /// Pipeline の結果 (failed 等)。
    pub status: String,
    pub is_mine: bool,
    pub needs_my_review: bool,
    pub is_draft: bool,
}

impl AzureMeta {
    pub(crate) fn from_candidate(candidate: &Candidate) -> Self {
        Self {
            kind: candidate.kind,
            work_item_type: candidate.work_item_type.clone(),
            status: candidate.status.clone(),
            is_mine: candidate.is_mine,
            needs_my_review: candidate.needs_my_review,
            is_draft: candidate.is_draft,
        }
    }

    /// 完了済みで、日常的には開かない候補か。行を薄く描き、順位を下げる。
    pub fn is_closed(&self) -> bool {
        let status = self.status.as_str();
        match self.kind {
            Kind::PullRequest => {
                status.eq_ignore_ascii_case("completed") || status.eq_ignore_ascii_case("abandoned")
            }
            Kind::WorkItem => ["closed", "done", "removed"]
                .iter()
                .any(|closed| status.eq_ignore_ascii_case(closed)),
            Kind::Pipeline | Kind::Project => false,
        }
    }

    pub fn is_failed_pipeline(&self) -> bool {
        self.kind == Kind::Pipeline && self.status.eq_ignore_ascii_case("failed")
    }

    /// タイトル一致度が同じ候補の並び順。小さいほど上位。
    /// 自分が関わる Active PR → その他の未完了 → 完了済みの順。
    pub(crate) fn state_rank(&self) -> u8 {
        if self.is_closed() {
            2
        } else if self.kind == Kind::PullRequest && self.is_mine {
            0
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(kind: Kind, status: &str, is_mine: bool) -> AzureMeta {
        AzureMeta {
            kind,
            work_item_type: None,
            status: status.to_string(),
            is_mine,
            needs_my_review: false,
            is_draft: false,
        }
    }

    #[test]
    fn closed_states_are_recognized_per_kind() {
        assert!(meta(Kind::PullRequest, "completed", false).is_closed());
        assert!(meta(Kind::PullRequest, "Abandoned", false).is_closed());
        assert!(!meta(Kind::PullRequest, "active", false).is_closed());
        assert!(meta(Kind::WorkItem, "Done", false).is_closed());
        assert!(meta(Kind::WorkItem, "Removed", false).is_closed());
        assert!(!meta(Kind::WorkItem, "Active", false).is_closed());
        assert!(!meta(Kind::Pipeline, "failed", false).is_closed());
    }

    #[test]
    fn state_rank_puts_my_active_prs_first_and_closed_items_last() {
        assert_eq!(meta(Kind::PullRequest, "active", true).state_rank(), 0);
        assert_eq!(meta(Kind::PullRequest, "active", false).state_rank(), 1);
        assert_eq!(meta(Kind::WorkItem, "New", false).state_rank(), 1);
        assert_eq!(meta(Kind::PullRequest, "completed", true).state_rank(), 2);
    }
}
