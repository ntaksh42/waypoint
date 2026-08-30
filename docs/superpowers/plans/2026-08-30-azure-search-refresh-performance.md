# Azure Search Refresh Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Azure 同期完了時の SQLite 読取をバックグラウンドへ移し、UI スレッドでは構築済み候補の交換だけを行う。

**Architecture:** `azure_devops::refresh_async` のワーカーが同期後にキャッシュ候補を読み、`RefreshReply` を Mutex 保護された一時領域へ保存してから通知する。トレイの UI スレッドは返信を `take` し、`Index` の Azure フィールドだけをメモリ上で再構築・交換する。返信を作れなかった場合は通知しても既存索引を保持する。既存の `REFRESHING` が同期を一つに直列化するため世代管理は追加せず、未消費の返信は最新完了結果で置換する。

**Tech Stack:** Rust 2024、Win32 `PostMessageW`、`std::sync::{Mutex, OnceLock}`、rusqlite、既存の `cargo test` ベンチ。

**Spec:** `docs/superpowers/specs/2026-08-30-search-performance-design.md`

## Global Constraints

- トリガーから描画までの予算は 50 ms。SQLite、Credential Manager、ネットワークアクセスを UI スレッドへ載せない。
- 検索結果の一致条件、順位、プレフィックス動作、空クエリの区分表示を変更しない。
- 更新失敗時は現在の Azure 検索索引を保持し、空の中間状態を公開しない。
- `Entry` と小文字キーは同じスナップショット内で構築し、長さと順序の対応を保つ。
- UI テキストは英語、コードコメントは日本語とする。
- 1 ファイル 500 行以下を維持し、今回の高速化と無関係な変更を行わない。
- 未追跡の `tests/perf_probe.rs` はユーザー所有として変更・削除・コミットしない。

---

### Task 1: キャッシュ読取結果を明示的な返信型にする

**Files:**
- Create: `src/azure_devops/candidate_cache.rs`
- Modify: `src/azure_devops/mod.rs:1-320` (候補読取関数を新モジュールへ移し、必要な項目だけ再公開)
- Test: `src/azure_devops/candidate_cache.rs`

**Interfaces:**
- Produces: `pub(crate) struct CachedCandidateGroups { pub pull_requests: Vec<Candidate>, pub work_items: Vec<Candidate> }`
- Produces: `pub(crate) fn try_cached_candidate_groups(settings: &AzureDevOpsSettings) -> Result<CachedCandidateGroups, String>`
- Preserves: `cached_candidate_groups(settings) -> (Vec<Candidate>, Vec<Candidate>)` for existing callers, implemented as a compatibility wrapper.

- [ ] **Step 1: Write the failing tests**

Add a pure result-composition helper so failure behavior is testable without the real shared database:

```rust
fn candidate(kind: Kind, name: &str) -> Candidate {
    Candidate {
        kind, status: String::new(), name: name.into(),
        detail: "Azure DevOps — org/project".into(),
        url: "https://dev.azure.com/org/project".into(),
        organization: "org".into(), project: "project".into(),
        aliases: Vec::new(), priority: 0, is_mine: false,
    }
}

#[test]
fn candidate_group_result_keeps_both_groups_when_cache_is_healthy() {
    let pull_requests = vec![candidate(Kind::PullRequest, "PR 1")];
    let work_items = vec![candidate(Kind::WorkItem, "42: Fix")];
    let groups = finish_candidate_groups(Ok((pull_requests.clone(), work_items.clone())), vec![])
        .unwrap();
    assert_eq!(groups.pull_requests, pull_requests);
    assert_eq!(groups.work_items, work_items);
}

#[test]
fn candidate_group_result_reports_shared_cache_failure() {
    let result = finish_candidate_groups(Err("shared cache unavailable".into()), vec![]);
    assert_eq!(result.unwrap_err(), "shared cache unavailable");
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test --lib azure_devops::tests::candidate_group_result
```

Expected: compilation fails because `finish_candidate_groups` and `CachedCandidateGroups` do not exist.

- [ ] **Step 3: Implement the result type and checked read**

Add:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CachedCandidateGroups {
    pub(crate) pull_requests: Vec<Candidate>,
    pub(crate) work_items: Vec<Candidate>,
}

fn finish_candidate_groups(
    shared: Result<(Vec<Candidate>, Vec<Candidate>), String>,
    history: Vec<Candidate>,
) -> Result<CachedCandidateGroups, String> {
    let (mut pull_requests, work_items) = shared?;
    pull_requests.extend(history);
    Ok(CachedCandidateGroups {
        pull_requests,
        work_items,
    })
}

pub(crate) fn try_cached_candidate_groups(
    settings: &AzureDevOpsSettings,
) -> Result<CachedCandidateGroups, String> {
    if !settings.enabled {
        return Ok(CachedCandidateGroups::default());
    }
    let shared = shared_cache::with_cached_connection(|connection| {
        let (groups, healthy) = read_candidate_groups(connection, settings);
        (
            healthy
                .then_some(groups)
                .ok_or_else(|| "Azure shared cache query failed.".to_string()),
            healthy,
        )
    })??;
    finish_candidate_groups(Ok(shared), pr_history_candidates(settings))
}
```

Keep the public compatibility wrapper's present fallback semantics for callers that intentionally tolerate missing caches:

```rust
pub(crate) fn cached_candidate_groups(
    settings: &AzureDevOpsSettings,
) -> (Vec<Candidate>, Vec<Candidate>) {
    try_cached_candidate_groups(settings)
        .map(|groups| (groups.pull_requests, groups.work_items))
        .unwrap_or_else(|_| (pr_history_candidates(settings), Vec::new()))
}
```

- [ ] **Step 4: Run focused and existing Azure tests**

Run:

```powershell
cargo test --lib azure_devops::tests
```

Expected: all Azure DevOps unit tests pass with zero warnings.

- [ ] **Step 5: Commit Task 1**

```powershell
git add src/azure_devops/candidate_cache.rs src/azure_devops/mod.rs
git commit -m "refactor(azure): expose checked cache candidate read"
```

---

### Task 2: 同期ワーカーから候補返信を渡す

**Files:**
- Create: `src/azure_devops/refresh.rs`
- Modify: `src/azure_devops/sync.rs:1-125` (定期同期処理を `refresh.rs` へ移す)
- Modify: `src/azure_devops/mod.rs:1-30` (`refresh_async`、`RefreshReply`、`take_refresh_reply` を再公開)
- Test: `src/azure_devops/refresh.rs`

**Interfaces:**
- Consumes: `try_cached_candidate_groups(&AzureDevOpsSettings) -> Result<CachedCandidateGroups, String>`
- Produces: `pub(crate) struct RefreshReply { pub candidates: Result<CachedCandidateGroups, String> }`
- Produces: `pub(crate) fn take_refresh_reply() -> Option<RefreshReply>`
- Changes: `refresh_async(settings, notify, message) -> bool` keeps its signature.

- [ ] **Step 1: Write failing mailbox tests**

Extract the mailbox operations so they can be tested synchronously:

```rust
#[test]
fn refresh_reply_mailbox_keeps_only_latest_completion() {
    clear_refresh_reply_for_test();
    store_refresh_reply(RefreshReply {
        candidates: Ok(CachedCandidateGroups::default()),
    });
    store_refresh_reply(RefreshReply {
        candidates: Err("latest".into()),
    });
    let reply = take_refresh_reply().unwrap();
    assert_eq!(reply.candidates.unwrap_err(), "latest");
}

#[test]
fn taking_refresh_reply_consumes_it() {
    clear_refresh_reply_for_test();
    store_refresh_reply(RefreshReply {
        candidates: Ok(CachedCandidateGroups::default()),
    });
    assert!(take_refresh_reply().is_some());
    assert!(take_refresh_reply().is_none());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test --lib azure_devops::refresh::tests::refresh_reply
```

Expected: compilation fails because the reply mailbox APIs do not exist.

- [ ] **Step 3: Implement the mailbox**

Use a poison-recovering `Mutex<Option<RefreshReply>>`. `REFRESHING` already
serializes accepted refreshes, so replacing the slot always keeps the latest completion:

```rust
pub(crate) struct RefreshReply {
    pub(crate) candidates: Result<CachedCandidateGroups, String>,
}

fn refresh_replies() -> &'static Mutex<Option<RefreshReply>> {
    static REPLIES: OnceLock<Mutex<Option<RefreshReply>>> = OnceLock::new();
    REPLIES.get_or_init(|| Mutex::new(None))
}

fn store_refresh_reply(reply: RefreshReply) {
    *lock_recovering(refresh_replies()) = Some(reply);
}

pub(crate) fn take_refresh_reply() -> Option<RefreshReply> {
    lock_recovering(refresh_replies()).take()
}
```

After network/cache writes finish and before `PostMessageW`, execute the expensive read on the same worker:

```rust
let candidates = super::try_cached_candidate_groups(&settings);
store_refresh_reply(RefreshReply {
    candidates,
});
```

- [ ] **Step 4: Run mailbox and Azure tests**

Run:

```powershell
cargo test --lib azure_devops::refresh::tests
cargo test --lib azure_devops::tests
```

Expected: all selected tests pass. Confirm no test reads the real Azure databases.

- [ ] **Step 5: Commit Task 2**

```powershell
git add src/azure_devops/refresh.rs src/azure_devops/sync.rs src/azure_devops/mod.rs
git commit -m "perf(azure): prepare search candidates off UI thread"
```

---

### Task 3: Index に構築済み候補を適用する

**Files:**
- Modify: `src/quick_launch/index.rs:140-215`
- Modify: `src/quick_launch/mod.rs:190-235` only if `AzureSnapshot` fields require module-level visibility
- Modify: `src/quick_launch/tests/index_test.rs:80-150,203-240`

**Interfaces:**
- Consumes: `CachedCandidateGroups { pull_requests, work_items }`
- Produces: `pub(crate) fn refresh_azure_candidates(&mut self, settings: &QuickLaunchSettings, groups: CachedCandidateGroups)`
- Preserves temporarily: `refresh_azure(&mut self, config: &Config)` for startup/tests until all callers migrate.

- [ ] **Step 1: Write a failing replacement test**

Replace the cache-dependent call in `refresh_azure_keeps_apps_bookmarks_history_and_folders` with explicit candidates and add an assertion that the new Azure data is searchable:

```rust
let groups = crate::azure_devops::CachedCandidateGroups {
    pull_requests: vec![azure_candidate(Kind::PullRequest, "PR 7: Faster refresh")],
    work_items: vec![azure_candidate(Kind::WorkItem, "42: Remove UI I/O")],
};
index.refresh_azure_candidates(&config.settings.quick_launch, groups);

assert_eq!(index.search("az faster refresh").len(), 1);
assert_eq!(index.search_cached_work_items("remove ui").len(), 1);
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test --lib quick_launch::tests::index_test::refresh_azure_keeps_apps_bookmarks_history_and_folders
```

Expected: compilation fails because `refresh_azure_candidates` does not exist.

- [ ] **Step 3: Extract candidate conversion and implement the replacement**

Refactor `azure_entries` so cache I/O and in-memory conversion are separate:

```rust
pub(crate) fn refresh_azure_candidates(
    &mut self,
    settings: &crate::config::QuickLaunchSettings,
    groups: crate::azure_devops::CachedCandidateGroups,
) {
    (self.azure, self.azure_work_items) = azure_entries_from_candidates(settings, groups);
    self.azure_work_items_lower =
        super::search::LowerKeys::build_for(&self.azure_work_items);
}
```

- [ ] **Step 4: Run all Quick Launch index/search tests**

Run:

```powershell
cargo test --lib quick_launch::tests::index_test
cargo test --lib quick_launch::tests::search_test
```

Expected: all selected tests pass and existing result ordering is unchanged.

- [ ] **Step 5: Commit Task 3**

```powershell
git add src/quick_launch/index.rs src/quick_launch/mod.rs src/quick_launch/tests/index_test.rs
git commit -m "refactor(quick-launch): apply Azure candidate snapshots"
```

---

### Task 4: UI通知を返信適用だけに変更する

**Files:**
- Modify: `src/tray/window.rs:145-157`
- Modify: `src/quick_launch_window/mod.rs:262-291`
- Modify: `src/quick_launch/tests/bench_io.rs:145-180`

**Interfaces:**
- Consumes: `azure_devops::take_refresh_reply() -> Option<RefreshReply>`
- Consumes: `Index::refresh_azure_candidates(settings, groups)`
- Changes: `quick_launch_window::configure_azure(config, groups)` accepts in-memory groups and performs no I/O.

- [ ] **Step 1: Add a benchmark that separates read from UI apply**

Change `bench_azure_refresh` to read one candidate snapshot before timing the UI-equivalent loop:

```rust
let groups = crate::azure_devops::try_cached_candidate_groups(azure_settings)
    .expect("Azure candidate cache must be readable for this machine benchmark");
let start = Instant::now();
for _ in 0..100 {
    index.refresh_azure_candidates(
        &config.settings.quick_launch,
        std::hint::black_box(groups.clone()),
    );
}
let apply_ms = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
println!("azure background cache read {cache_read:>8.3} ms  UI snapshot apply {apply_ms:>8.3} ms");
assert!(apply_ms < 50.0, "UI snapshot apply exceeded 50 ms: {apply_ms:.3} ms");
```

- [ ] **Step 2: Run the benchmark before migrating the UI caller**

Run:

```powershell
cargo test --release --lib -- --ignored --nocapture bench_azure_refresh
```

Expected: benchmark builds and reports the in-memory apply cost. If it exceeds 50 ms, stop and profile conversion before changing the UI path.

- [ ] **Step 3: Migrate the UI message handler**

Change the handler to consume exactly one completed reply:

```rust
WM_AZURE_DEVOPS_REFRESHED => {
    if let Some(reply) = crate::azure_devops::take_refresh_reply() {
        match reply.candidates {
            Ok(groups) => with_state(|state| {
                let state = state.borrow();
                if let Some(state) = state.as_ref() {
                    quick_launch_window::configure_azure(&state.config, groups);
                }
            }),
            Err(error) => crate::panic_log::record(&format!(
                "azure devops: candidate refresh failed: {error}"
            )),
        }
    }
    LRESULT(0)
}
```

Change `configure_azure` to accept `CachedCandidateGroups` and call only `refresh_azure_candidates`. It must still reset `previous_query` and redraw an already visible window after a successful swap. A missing or failed reply must not call it.

- [ ] **Step 4: Run focused tests and the machine benchmark**

Run:

```powershell
cargo test --lib quick_launch::tests::index_test
cargo test --release --lib -- --ignored --nocapture bench_azure_refresh
```

Expected: tests pass; output shows the cache read remains on the background side and UI snapshot apply is below 50 ms.

- [ ] **Step 5: Commit Task 4**

```powershell
git add src/tray/window.rs src/quick_launch_window/mod.rs src/quick_launch/tests/bench_io.rs
git commit -m "perf(quick-launch): remove Azure cache I/O from UI refresh"
```

---

### Task 5: 全体回帰検証と次のボトルネック判定

**Files:**
- Modify if measurements changed: `docs/superpowers/specs/2026-08-30-search-performance-design.md`
- Do not modify: `tests/perf_probe.rs`

**Interfaces:**
- Consumes the completed Azure snapshot flow from Tasks 1-4.
- Produces benchmark evidence and a decision on whether another implementation plan is required.

- [ ] **Step 1: Format and inspect the exact diff**

Run:

```powershell
cargo fmt --all
git diff --check
git status --short
git diff --stat
```

Expected: no formatting errors; `tests/perf_probe.rs` remains untracked and absent from staged changes.

- [ ] **Step 2: Stop the running application and run all correctness gates**

Run:

```powershell
Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
cargo fmt --all -- --check
```

Expected: every command exits 0 with zero warnings.

- [ ] **Step 3: Run the complete release benchmark suite**

Run:

```powershell
cargo test --release --lib -- --ignored --nocapture bench_
```

Expected: all ignored benchmarks pass. Record at minimum real-config search, sections, Azure background read, Azure UI apply, `Index::build`, `apps::scan`, browser history scan, and menu build values.

- [ ] **Step 4: Audit remaining search-experience costs**

Compare the new output with the baseline in the spec. Apply these decision rules:

- If any UI-thread or trigger-path operation is at least 1 ms and avoidably repeated, create a focused follow-up plan before implementation.
- If a background/startup operation exceeds 50 ms but does not block display, only optimize it when it delays candidate availability or repeats unnecessarily.
- Do not change 0.1 ms-scale synchronous search unless a larger synthetic dataset demonstrates at least a 20% improvement without result changes.
- Record measured no-op candidates so they are not retried.

- [ ] **Step 5: Commit measurement documentation if changed**

```powershell
git add docs/superpowers/specs/2026-08-30-search-performance-design.md
git commit -m "docs: record search performance results"
```

- [ ] **Step 6: Push only after the completion audit**

Verify the branch and outgoing commits:

```powershell
git status --short --branch
git log --oneline origin/main..HEAD
git diff --check origin/main..HEAD
```

Expected: only intended files and commits are outgoing; `tests/perf_probe.rs` remains untracked. Then run:

```powershell
git push origin main
git status --short --branch
git log -1 --oneline origin/main
```

Expected: push succeeds and `main` is synchronized with `origin/main` while the user-owned untracked file remains untouched.
