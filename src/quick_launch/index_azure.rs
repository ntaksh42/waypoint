//! `Index` の Azure DevOps 候補の組み立て。

use super::Entry;
use super::azure::AzureIndexed;
use super::azure_entries::azure_candidate_entry;

/// Azure DevOps キャッシュを読み、検索用候補へ変換する起動時経路。
pub(super) fn azure_entries(
    settings: &crate::config::QuickLaunchSettings,
) -> (Vec<AzureIndexed>, Vec<Entry>) {
    let (pull_requests, work_items) =
        crate::azure_devops::cached_candidate_groups(&settings.azure_devops);
    azure_entries_from_candidates(
        &settings.azure_devops,
        crate::azure_devops::CachedCandidateGroups {
            pull_requests,
            work_items,
        },
    )
}

/// 同期完了時に UI スレッドへ渡す、検索用に変換済みの Azure 候補。
/// 数千件ぶんの `Entry` / `LowerKeys` の組み立てを、同期を行った
/// バックグラウンドスレッドで済ませるための入れ物 (`build_azure`)。
#[derive(Debug, Default)]
pub(crate) struct AzureBuilt {
    pub(super) azure: Vec<AzureIndexed>,
    pub(super) work_items: Vec<Entry>,
    pub(super) work_items_lower: Vec<super::search::LowerKeys>,
}

/// メモリ上の Azure DevOps 候補を、索引へそのまま差し替えられる形へ変換する。
pub(crate) fn build_azure(
    settings: &crate::config::AzureDevOpsSettings,
    groups: crate::azure_devops::CachedCandidateGroups,
) -> AzureBuilt {
    let (azure, work_items) = azure_entries_from_candidates(settings, groups);
    let work_items_lower = super::search::LowerKeys::build_for(&work_items);
    AzureBuilt {
        azure,
        work_items,
        work_items_lower,
    }
}

/// メモリ上の Azure DevOps 候補を検索用の索引へ変換する。
pub(super) fn azure_entries_from_candidates(
    settings: &crate::config::AzureDevOpsSettings,
    groups: crate::azure_devops::CachedCandidateGroups,
) -> (Vec<AzureIndexed>, Vec<Entry>) {
    let mut candidates = if settings.enabled {
        crate::azure_devops::project_candidates(settings)
    } else {
        Vec::new()
    };
    candidates.extend(groups.pull_requests);
    // 優先度を最優先しつつ、同一プロジェクト内では自分が関与する PR、
    // 日常的に開く Active PR、失敗した Pipeline の順に先頭へ置く。
    // 通常の使用履歴ランキングも後段で効く。
    candidates.sort_by_key(|candidate| (candidate.priority, azure_urgency(candidate)));
    let indexed = candidates
        .into_iter()
        .map(|candidate| {
            let entry = azure_candidate_entry(candidate.clone());
            AzureIndexed {
                lower: super::search::LowerKeys::new(&entry),
                entry,
                kind: candidate.kind,
                status: candidate.status,
                is_mine: candidate.is_mine,
                is_author: candidate.is_author,
                is_reviewer: candidate.is_reviewer,
                needs_my_review: candidate.needs_my_review,
                waiting_for_others: candidate.waiting_for_others,
                is_draft: candidate.is_draft,
                ready_to_complete: candidate.ready_to_complete,
                is_stale: candidate.is_stale,
            }
        })
        .collect();
    let work_items = groups
        .work_items
        .into_iter()
        .map(azure_candidate_entry)
        .collect();
    (indexed, work_items)
}

/// 自分が関与する PR、Active な PR、失敗した Pipeline の順に小さい値を返す。
/// `azure_candidates.sort_by_key` の第二キーとして使う。
fn azure_urgency(candidate: &crate::azure_devops::Candidate) -> u8 {
    match (&candidate.kind, candidate.status.as_str()) {
        (crate::azure_devops::Kind::PullRequest, _) if candidate.is_mine => 0,
        (crate::azure_devops::Kind::PullRequest, status)
            if status.eq_ignore_ascii_case("active") =>
        {
            1
        }
        (crate::azure_devops::Kind::Pipeline, status) if status.eq_ignore_ascii_case("failed") => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::azure_urgency;
    use crate::azure_devops::{Candidate, Kind};

    fn candidate(kind: Kind, status: &str, is_mine: bool) -> Candidate {
        Candidate {
            work_item_type: None,
            kind,
            status: status.to_string(),
            name: String::new(),
            detail: String::new(),
            branch: None,
            url: String::new(),
            organization: String::new(),
            project: String::new(),
            aliases: Vec::new(),
            priority: 0,
            is_mine,
            is_author: false,
            is_reviewer: false,
            needs_my_review: false,
            waiting_for_others: false,
            is_draft: false,
            ready_to_complete: false,
            is_stale: false,
        }
    }

    #[test]
    fn own_pull_requests_rank_before_other_active_pull_requests() {
        let mine = candidate(Kind::PullRequest, "active", true);
        let others_active = candidate(Kind::PullRequest, "active", false);
        assert!(azure_urgency(&mine) < azure_urgency(&others_active));
    }

    #[test]
    fn own_completed_pull_request_still_ranks_before_active_ones_from_others() {
        let mine_completed = candidate(Kind::PullRequest, "completed", true);
        let others_active = candidate(Kind::PullRequest, "active", false);
        assert!(azure_urgency(&mine_completed) < azure_urgency(&others_active));
    }

    #[test]
    fn active_pull_requests_rank_before_failed_pipelines() {
        let active_pr = candidate(Kind::PullRequest, "active", false);
        let failed_pipeline = candidate(Kind::Pipeline, "failed", false);
        assert!(azure_urgency(&active_pr) < azure_urgency(&failed_pipeline));
    }
}
