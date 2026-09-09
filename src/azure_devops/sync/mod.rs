//! バックグラウンド同期: PR 履歴 (Completed/Abandoned) のキャッシュ更新と、
//! `az wit ` / `az pr` / `az pipeline ` のライブ検索。

mod activity;
mod common;
mod pipelines;
mod pull_requests;
mod work_items;

pub use common::refresh_async;
pub(crate) use common::{REFRESHING, take_refresh_reply};

pub use activity::{ProjectActivity, apply_optimization, suggest_priorities_async};
pub use pipelines::{search_pipelines_live_async, take_pipeline_results};
pub use pull_requests::{
    PullRequestReply, search_pull_requests_live_async, take_pull_request_results,
};
pub use work_items::{WorkItemReply, search_work_items_async, take_work_item_results};
