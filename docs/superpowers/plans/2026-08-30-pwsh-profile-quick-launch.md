# Quick Launch `pf ` (PowerShell PROFILE 実行) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Quick Launch に `pf ` プレフィックスを追加し、PowerShell 7 (`pwsh`) の `$PROFILE` が定義した関数・エイリアスを検索し、引数付きで Windows Terminal 上に即実行できるようにする。

**Architecture:** `pwsh_profile.rs`(新規)が waypoint 起動時にバックグラウンドスレッドで `pwsh` を 2 回起動し(`-NoProfile` のベースラインと通常起動)、差分から PROFILE 自体が定義した関数・エイリアス名だけを取り出す。結果は `tabs`(`t ` プレフィックス)と同じパターンで `quick_launch_window::STATE` にキャッシュし、`Index::set_pwsh_functions` で都度再適用する。検索はキー入力のたびに変わる引数を候補ラベル・実行コマンドへ連結する必要があるため、Everything / Azure ライブ検索と同様に `Index::search` の同期モデルには乗せず、専用メソッド `Index::search_pwsh_functions` を window 層から直接呼ぶ。実行は既存の `shell::open_terminal`(`ps ` プレフィックス)と同じ `wt.exe <pwsh> -NoExit -Command <command>` パターンを踏襲する。

**Tech Stack:** Rust、`windows` crate (Win32)、`std::process::Command`(pwsh 起動)、egui/eframe(設定画面のトグルのみ)。

**Spec:** `docs/superpowers/specs/2026-08-30-pwsh-profile-quick-launch-design.md`(設計)、`docs/spec.md` FR-9.19 と 02 章の該当行(要件)。実行者は両方を読むこと。

## Global Constraints

- UI テキストは英語、コードコメントは日本語 (AGENTS.md)
- 1 ファイルは 500 行以下に保つ (AGENTS.md)
- 警告 0 を維持する。各タスクの完了条件は `cargo clippy --all-targets -- -D warnings` が通ること (AGENTS.md)
- リビルド前に実行中の `waypoint` を終了する: `Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force`
- `pf ` は PowerShell 7 (`pwsh`) の `$PROFILE` のみが対象。Windows PowerShell 5.1 へのフォールバックはしない(FR-9.19)
- PROFILE の関数・エイリアス一覧は waypoint 起動時に一度だけ非同期取得し、以後は設定保存のたびに再スキャンしない(FR-9.19)
- `RunPwshCommand` は Recent/Frequent の使用履歴に記録しない(引数を含め毎回組み合わせが変わるため)

---

### Task 1: `shell::run_pwsh_command` — PROFILE コマンドの実行

**Files:**
- Modify: `src/shell.rs`

**Interfaces:**
- Produces: `pub(crate) fn find_pwsh() -> Option<PathBuf>`(既存 `fn find_pwsh` の可視性変更)、`pub fn run_pwsh_command(command: &str) -> std::io::Result<()>`

- [ ] **Step 1: `find_pwsh` を crate 内から呼べるようにする**

`src/shell.rs` の既存の `fn find_pwsh() -> Option<PathBuf> {` を `pub(crate) fn find_pwsh() -> Option<PathBuf> {` に変更する(後続タスクの `pwsh_profile.rs` から呼ぶため)。

- [ ] **Step 2: `run_pwsh_command` を追加する**

`open_terminal` の直後(`find_pwsh` の直前)に追加:

```rust
/// `pf ` プレフィックスで選んだ関数・エイリアスを、引数を含めた文字列の
/// まま Windows Terminal + PowerShell 7 で実行する (FR-9.19)。
///
/// `open_terminal` と同じ理由で `pwsh.exe` のフルパスを渡す。`pf ` の検索
/// 結果は `pwsh` が見つかるときしか存在しないため、通常はここで
/// `NotFound` にはならない。
pub fn run_pwsh_command(command: &str) -> std::io::Result<()> {
    let Some(pwsh) = find_pwsh() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "pwsh.exe not found",
        ));
    };
    std::process::Command::new("wt.exe")
        .arg(&pwsh)
        .args(["-NoExit", "-Command", command])
        .spawn()
        .map(|_| ())
}
```

- [ ] **Step 3: ビルド確認**

Run: `cargo build`
Expected: 成功(既存の `open_terminal` を呼ぶコードは変更していないので警告なし)

- [ ] **Step 4: Commit**

```bash
git add src/shell.rs
git commit -m "$(cat <<'EOF'
feat(shell): pf 用の run_pwsh_command を追加

pf プレフィックス (次タスク以降) が選んだ PROFILE 関数・エイリアスを
Windows Terminal + PowerShell 7 で実行するためのヘルパー。open_terminal
と同じ pwsh フルパス解決を再利用する。
EOF
)"
```

---

### Task 2: `pwsh_profile.rs` — PROFILE 差分診断モジュール

**Files:**
- Create: `src/pwsh_profile.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::shell::find_pwsh() -> Option<PathBuf>`(Task 1)
- Produces: `pub struct ProfileCommand { pub name: String }`、`pub fn scan() -> Vec<ProfileCommand>`、`pub fn scan_async(notify: HWND, message: u32)`、`pub fn take_scanned() -> Vec<ProfileCommand>`

- [ ] **Step 1: モジュールを登録する**

`src/lib.rs` の `pub mod process;` の直後、`pub mod quick_launch;` の直前に追加:

```rust
pub mod pwsh_profile;
```

- [ ] **Step 2: 失敗するテストを書く(差分ロジック)**

`src/pwsh_profile.rs` を新規作成:

```rust
//! PowerShell 7 (`pwsh`) の `$PROFILE` が定義した関数・エイリアスの一覧化
//! (`pf ` プレフィックス、FR-9.19)。
//!
//! `Get-Command` は `Import-Module` されたモジュール公開分や、素の
//! PowerShell エンジンが内蔵するエイリアス (`ls` / `cd` 等) も返してしまう。
//! `$PROFILE` 自体が定義したものだけに絞るため、`-NoProfile` のベースラインと
//! 通常起動の差分を取る。waypoint 起動時にバックグラウンドスレッドで一度だけ
//! 実行し (`scan_async`)、結果はメモリにキャッシュする (再スキャンしない)。

use std::collections::HashSet;

/// `$PROFILE` が定義した関数またはエイリアス 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCommand {
    pub name: String,
}

/// `full` から `baseline` に含まれる名前 (エンジン内蔵・モジュール提供) を
/// 除いた、`$PROFILE` 自体が定義した名前だけを返す。大文字小文字を無視して
/// 重複排除し、名前順に並べる。
fn diff_profile_commands(baseline: &[String], full: &[String]) -> Vec<ProfileCommand> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(commands: &[ProfileCommand]) -> Vec<&str> {
        commands.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn diff_excludes_engine_builtins_and_modules() {
        let baseline = vec![
            "ls".to_string(),
            "cd".to_string(),
            "Get-ChildItem".to_string(),
        ];
        let full = vec![
            "ls".to_string(),
            "cd".to_string(),
            "Get-ChildItem".to_string(),
            "Invoke-Build".to_string(), // PROFILE で定義
        ];
        assert_eq!(
            names(&diff_profile_commands(&baseline, &full)),
            vec!["Invoke-Build"]
        );
    }

    #[test]
    fn diff_dedupes_case_insensitively_and_sorts_by_name() {
        let baseline: Vec<String> = vec![];
        let full = vec![
            "Zeta-Tool".to_string(),
            "alpha-tool".to_string(),
            "Alpha-Tool".to_string(), // 大文字小文字違いの重複
        ];
        assert_eq!(
            names(&diff_profile_commands(&baseline, &full)),
            vec!["alpha-tool", "Zeta-Tool"]
        );
    }

    #[test]
    fn diff_ignores_blank_lines() {
        let baseline: Vec<String> = vec![];
        let full = vec![
            "".to_string(),
            "  ".to_string(),
            "Real-Function".to_string(),
        ];
        assert_eq!(
            names(&diff_profile_commands(&baseline, &full)),
            vec!["Real-Function"]
        );
    }

    #[test]
    fn diff_returns_empty_when_profile_defines_nothing_new() {
        let baseline = vec!["ls".to_string()];
        let full = vec!["ls".to_string()];
        assert!(diff_profile_commands(&baseline, &full).is_empty());
    }
}
```

- [ ] **Step 3: テストが失敗することを確認する**

Run: `cargo test --lib pwsh_profile::tests`
Expected: `todo!()` により全テストが FAIL (panic "not yet implemented")

- [ ] **Step 4: 差分ロジックを実装する**

`diff_profile_commands` を実装(`todo!()` を置き換え):

```rust
fn diff_profile_commands(baseline: &[String], full: &[String]) -> Vec<ProfileCommand> {
    let baseline_lower: HashSet<String> =
        baseline.iter().map(|name| name.to_lowercase()).collect();
    let mut seen = HashSet::new();
    let mut commands: Vec<ProfileCommand> = full
        .iter()
        .filter(|name| !name.trim().is_empty())
        .filter_map(|name| {
            let lower = name.to_lowercase();
            if baseline_lower.contains(&lower) || !seen.insert(lower) {
                return None;
            }
            Some(ProfileCommand { name: name.clone() })
        })
        .collect();
    commands.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    commands
}
```

- [ ] **Step 5: テストが通ることを確認する**

Run: `cargo test --lib pwsh_profile::tests`
Expected: PASS (4 tests)

- [ ] **Step 6: `pwsh` 起動・非同期スキャンを実装する(テスト対象外の Win32/プロセス起動部分)**

`diff_profile_commands` の前に追加(定数・struct 定義は import の直後):

```rust
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// コンソールウィンドウを表示せずにプロセスを起動する (`CREATE_NO_WINDOW`)。
/// waypoint は `windows_subsystem = "windows"` でコンソールを持たないため、
/// 指定しないと `pwsh.exe` 起動のたびに黒いウィンドウが一瞬フラッシュする。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const LIST_COMMANDS_SCRIPT: &str =
    "Get-Command -CommandType Function,Alias | Select-Object -ExpandProperty Name";

static SCAN_RESULT: Mutex<Vec<ProfileCommand>> = Mutex::new(Vec::new());
```

`diff_profile_commands` の後、`#[cfg(test)]` の前に追加:

```rust
/// `pwsh` を 2 回起動して差分診断する。`pwsh` が見つからなければ空を返す
/// (Everything 未検出と同じくエラー表示はしない)。
pub fn scan() -> Vec<ProfileCommand> {
    let Some(pwsh) = crate::shell::find_pwsh() else {
        return Vec::new();
    };
    let Some(baseline) = run_get_command(&pwsh, true) else {
        return Vec::new();
    };
    let Some(full) = run_get_command(&pwsh, false) else {
        return Vec::new();
    };
    diff_profile_commands(&baseline, &full)
}

/// バックグラウンドスレッドで `scan()` を実行し、完了したら `notify` へ
/// `message` を送る。結果は `take_scanned` で取り出すまでメモリに残る。
pub fn scan_async(notify: HWND, message: u32) {
    let notify = notify.0 as isize;
    thread::spawn(move || {
        let commands = scan();
        if let Ok(mut guard) = SCAN_RESULT.lock() {
            *guard = commands;
        }
        unsafe {
            let _ = PostMessageW(Some(HWND(notify as *mut _)), message, WPARAM(0), LPARAM(0));
        }
    });
}

/// `scan_async` が残した結果を取り出す (呼び出し後は空になる)。
pub fn take_scanned() -> Vec<ProfileCommand> {
    SCAN_RESULT
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default()
}

/// `pwsh` を起動して関数・エイリアス名を 1 行 1 件で取得する。
/// `no_profile` が真ならベースライン (`-NoProfile`) を取る。
fn run_get_command(pwsh: &Path, no_profile: bool) -> Option<Vec<String>> {
    let mut command = Command::new(pwsh);
    command.creation_flags(CREATE_NO_WINDOW);
    command.stdin(Stdio::null());
    if no_profile {
        command.arg("-NoProfile");
    }
    command.args(["-NoLogo", "-Command", LIST_COMMANDS_SCRIPT]);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    )
}
```

- [ ] **Step 7: ビルドとテストを確認する**

Run: `cargo build && cargo test --lib pwsh_profile`
Expected: ビルド成功、4 テスト PASS(`scan` / `scan_async` / `run_get_command` は実プロセス起動を伴うため単体テスト対象外 — `open_terminal` と同じ扱い、手動確認は Task 8)

- [ ] **Step 8: clippy 確認**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0(`scan`/`scan_async`/`take_scanned`/`ProfileCommand` は `pub` なので、まだ他から呼ばれていなくても dead_code にはならない)

- [ ] **Step 9: Commit**

```bash
git add src/pwsh_profile.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat(pwsh-profile): PROFILE 関数・エイリアスの差分診断を追加

pwsh を -NoProfile / 通常起動の2回動かし、Get-Command の差分から
PROFILE 自体が定義した関数・エイリアスだけを取り出す。モジュール提供分と
エンジン内蔵エイリアス (ls/cd 等) を除外する差分ロジックをテストで固定する。
非同期スキャン (scan_async/take_scanned) はまだどこからも呼ばれない
(Quick Launch への配線は後続タスク)。
EOF
)"
```

---

### Task 3: 設定 `include_pwsh_functions`

**Files:**
- Modify: `src/config/mod.rs`
- Modify: `src/settings_main/trigger_draft.rs`
- Modify: `src/settings_main/ui_trigger.rs`

**Interfaces:**
- Produces: `QuickLaunchSettings.include_pwsh_functions: bool`(既定 `true`)

- [ ] **Step 1: 設定フィールドを追加する**

`src/config/mod.rs` の `QuickLaunchSettings` に、`include_everything` の直後へ追加:

```rust
    #[serde(default = "default_true")]
    pub include_everything: bool,
    /// PowerShell PROFILE (`$PROFILE`) の関数・エイリアスを Quick Launch から
    /// 検索・実行する (`pf `、FR-9.19)。waypoint 起動時にバックグラウンドで
    /// 一度だけ取得するだけで、この設定自体はキー入力経路に影響しない
    #[serde(default = "default_true")]
    pub include_pwsh_functions: bool,
```

`impl Default for QuickLaunchSettings` の `include_everything: true,` の直後へ追加:

```rust
            include_everything: true,
            include_pwsh_functions: true,
```

- [ ] **Step 2: 設定画面のドラフト状態に追加する**

`src/settings_main/trigger_draft.rs` の `TriggerDraft` 構造体、`include_everything: bool,` の直後へ追加:

```rust
    pub(super) include_everything: bool,
    pub(super) include_pwsh_functions: bool,
```

`from_config` の `include_everything: quick_launch.include_everything,` の直後へ追加:

```rust
            include_everything: quick_launch.include_everything,
            include_pwsh_functions: quick_launch.include_pwsh_functions,
```

- [ ] **Step 3: チェックボックスを追加する**

`src/settings_main/ui_trigger.rs` の Everything のチェックボックスの直後へ追加:

```rust
                        ui.checkbox(
                            &mut draft.include_everything,
                            "Search all files via Everything (type \"f \" to search; requires Everything to be running)",
                        );
                        ui.checkbox(
                            &mut draft.include_pwsh_functions,
                            "Run PowerShell PROFILE functions (type \"pf \" to search; scanned once at startup)",
                        );
```

保存処理(`apply` ブロック)の `self.config.settings.quick_launch.include_everything = draft.include_everything;` の直後へ追加:

```rust
                self.config.settings.quick_launch.include_everything = draft.include_everything;
                self.config.settings.quick_launch.include_pwsh_functions =
                    draft.include_pwsh_functions;
```

- [ ] **Step 4: ビルド確認**

Run: `cargo build`
Expected: 成功(新フィールドは `#[serde(default = ...)]` 付きなので既存の `config.json` の読み込みも壊れない)

- [ ] **Step 5: Commit**

```bash
git add src/config/mod.rs src/settings_main/trigger_draft.rs src/settings_main/ui_trigger.rs
git commit -m "$(cat <<'EOF'
feat(config): pf 用の include_pwsh_functions 設定を追加

既定オン。設定画面の Quick Launch タブにトグルを追加する。
EOF
)"
```

---

### Task 4: `Action::RunPwshCommand` とプレフィックス判定

**Files:**
- Modify: `src/quick_launch/mod.rs`
- Modify: `src/quick_launch_history.rs`
- Modify: `src/quick_launch_window/badge.rs`
- Modify: `src/quick_launch_window/draw.rs`
- Modify: `src/tray/window.rs`
- Create: `src/quick_launch/tests/pwsh_test.rs`
- Modify: `src/quick_launch/tests/mod.rs`

**Interfaces:**
- Consumes: `shell::run_pwsh_command`(Task 1)
- Produces: `Action::RunPwshCommand(String)`、`pub const PWSH_PROFILE_PREFIX: &str = "pf ";`、`pub(crate) fn split_pwsh_command(rest: &str) -> (&str, &str)`

`Action` は `PartialEq, Eq` を derive した enum なので、新しいバリアントを追加すると
既存の全 `match` が非網羅になりコンパイルが壊れる。**このタスクでは変更対象の
5 箇所すべてを同時に直す**(shell.rs は Task 1 で既に対応済み)。

- [ ] **Step 1: 失敗するテストを書く**

`src/quick_launch/tests/mod.rs` に `mod pwsh_test;` を追加(`mod index_test;` の直後、アルファベット順):

```rust
mod convert_test;
mod fuzzy_test;
mod index_test;
mod pwsh_test;
mod search_test;
```

`src/quick_launch/tests/pwsh_test.rs` を新規作成:

```rust
//! `pf ` プレフィックス — PowerShell PROFILE 関数・エイリアスの検索・実行。

use super::super::*;

#[test]
fn split_pwsh_command_splits_on_first_space_only() {
    assert_eq!(split_pwsh_command("Get-MyThing"), ("Get-MyThing", ""));
    assert_eq!(
        split_pwsh_command("Get-MyThing -Foo bar"),
        ("Get-MyThing", "-Foo bar")
    );
    assert_eq!(split_pwsh_command(""), ("", ""));
}

#[test]
fn prefix_badge_recognizes_pf_prefix() {
    assert_eq!(prefix_badge("pf "), Some("PWSH"));
    assert_eq!(prefix_badge("pf Get-MyThing"), Some("PWSH"));
    assert_eq!(prefix_badge("ps "), Some("TERMINAL"));
}

#[test]
fn run_pwsh_command_entries_are_not_saved_as_favorites() {
    let entry = Entry {
        name: "Get-MyThing".to_string(),
        breadcrumb: "PowerShell PROFILE".to_string(),
        path: String::new(),
        action: Action::RunPwshCommand("Get-MyThing".to_string()),
        branch: None,
    };
    assert_eq!(entry.to_item(), None);
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test --lib quick_launch::tests::pwsh_test`
Expected: コンパイルエラー(`split_pwsh_command` / `PWSH_PROFILE_PREFIX` / `Action::RunPwshCommand` が未定義)

- [ ] **Step 3: `Action` にバリアントを追加する**

`src/quick_launch/mod.rs` の `Action` enum、`AzureSuggestPriorities,` の直後(`}` の前)へ追加:

```rust
    AzureSuggestPriorities,
    /// `pf ` プレフィックスで選んだ関数・エイリアスを、引数を含めた文字列の
    /// まま Windows Terminal + PowerShell 7 で実行する (FR-9.19)。
    RunPwshCommand(String),
}
```

- [ ] **Step 4: `Entry::to_item` を直す**

同ファイルの `Entry::to_item` の `None` を返す arm に追加:

```rust
            Action::FocusWindow(_)
            | Action::FocusBrowserTab(_)
            | Action::OpenUrl(_)
            | Action::OpenInTerminal
            | Action::ReplaceQuery(_)
            | Action::AzureLiveWorkItemSearch(_)
            | Action::AzureLivePullRequestSearch { .. }
            | Action::AzureLivePipelineSearch { .. }
            | Action::AzureSuggestPriorities
            | Action::RunPwshCommand(_) => None,
```

- [ ] **Step 5: プレフィックス定数と `prefix_badge` を追加する**

`TERMINAL_PREFIX` の定義の直後へ追加:

```rust
/// フォルダをターミナルで開く検索モードに入るプレフィックス (末尾の半角スペース込み)。
const TERMINAL_PREFIX: &str = "ps ";
/// PowerShell PROFILE の関数・エイリアスを検索・実行するプレフィックス
/// (末尾の半角スペース込み)。`ps ` (フォルダをターミナルで開く) とは別物。
pub const PWSH_PROFILE_PREFIX: &str = "pf ";
```

`prefix_badge` 内、`TERMINAL_PREFIX` の分岐の直後へ追加:

```rust
    } else if query.starts_with(TERMINAL_PREFIX) {
        Some("TERMINAL")
    } else if query.starts_with(PWSH_PROFILE_PREFIX) {
        Some("PWSH")
    } else if query.starts_with(EVERYTHING_PREFIX) {
```

- [ ] **Step 6: `split_pwsh_command` を追加する**

`effective_search_term` の直後へ追加:

```rust
/// `pf ` プレフィックスの残り文字列を、関数・エイリアス名のトークンと
/// それに続く実行時引数へ分割する。半角スペースが無ければ全体を名前側とする。
pub(crate) fn split_pwsh_command(rest: &str) -> (&str, &str) {
    match rest.split_once(' ') {
        Some((name, args)) => (name, args),
        None => (rest, ""),
    }
}
```

- [ ] **Step 7: `quick_launch_history::key_kind` を直す**

`src/quick_launch_history.rs` の `key_kind` の `None` を返す arm に追加:

```rust
        Action::FocusWindow(_)
        | Action::FocusBrowserTab(_)
        | Action::ReplaceQuery(_)
        | Action::AzureLiveWorkItemSearch(_)
        | Action::AzureLivePullRequestSearch { .. }
        | Action::AzureLivePipelineSearch { .. }
        | Action::AzureSuggestPriorities
        | Action::RunPwshCommand(_) => None,
```

- [ ] **Step 8: `quick_launch_window::badge` を直す**

`src/quick_launch_window/badge.rs` の `badge_color` に `"PWSH"` を追加:

```rust
pub(super) fn badge_color(badge: &str) -> COLORREF {
    match badge {
        "WINDOWS" | "APPS" | "TERMINAL" | "PWSH" => rgb(143, 168, 118), // 緑寄りの寒色
        "BOOKMARKS" | "HISTORY" | "FILES" | "TABS" | "AZURE DEVOPS" => rgb(95, 157, 176), // 青寄りの寒色
        _ => ACCENT,
    }
}
```

`action_color` の `ACCENT` を返す arm に追加:

```rust
pub(super) fn action_color(action: &Action) -> COLORREF {
    match action {
        Action::FocusWindow(_) | Action::LaunchApp => badge_color("WINDOWS"),
        Action::OpenFolder(_)
        | Action::OpenUrl(_)
        | Action::FocusBrowserTab(_)
        | Action::OpenWithDefaultHandler
        | Action::OpenInTerminal
        | Action::ReplaceQuery(_)
        | Action::AzureLiveWorkItemSearch(_)
        | Action::AzureLivePullRequestSearch { .. }
        | Action::AzureLivePipelineSearch { .. }
        | Action::AzureSuggestPriorities
        | Action::RunPwshCommand(_) => ACCENT,
    }
}
```

- [ ] **Step 9: `quick_launch_window::draw` を直す**

`src/quick_launch_window/draw.rs` のアイコン選択 `match entry.action` で、`draw_command_icon` を呼ぶ arm に追加:

```rust
                Action::ReplaceQuery(_)
                | Action::AzureLiveWorkItemSearch(_)
                | Action::AzureLivePullRequestSearch { .. }
                | Action::AzureLivePipelineSearch { .. }
                | Action::AzureSuggestPriorities
                | Action::RunPwshCommand(_) => {
                    draw_command_icon(draw.hDC, draw.rcItem, dpi, name_font)
                }
```

- [ ] **Step 10: `tray/window.rs` の実行ディスパッチを直す**

`src/tray/window.rs` の `WM_QUICK_LAUNCH_EXECUTE` 内、`quick_launch::Action::OpenInTerminal => { ... }` の直後へ追加:

```rust
                    quick_launch::Action::OpenInTerminal => {
                        let _ = shell::open_terminal(&entry.path);
                    }
                    quick_launch::Action::RunPwshCommand(command) => {
                        let _ = shell::run_pwsh_command(&command);
                    }
```

- [ ] **Step 11: テストとビルドを確認する**

Run: `cargo build && cargo test --lib`
Expected: 全テスト PASS(既存テストも壊れていないこと)

- [ ] **Step 12: clippy 確認**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0

- [ ] **Step 13: Commit**

```bash
git add src/quick_launch/mod.rs src/quick_launch_history.rs src/quick_launch_window/badge.rs \
        src/quick_launch_window/draw.rs src/tray/window.rs \
        src/quick_launch/tests/mod.rs src/quick_launch/tests/pwsh_test.rs
git commit -m "$(cat <<'EOF'
feat(quick-launch): Action::RunPwshCommand と pf プレフィックス判定を追加

pf の badge/実行ディスパッチ/お気に入り除外/履歴除外を配線する。検索
インデックス自体 (Index::search_pwsh_functions) は次タスクで追加するため、
この時点では pf と入力しても常に 0 件になる。
EOF
)"
```

---

### Task 5: `Index` — PROFILE 候補の保持と検索

**Files:**
- Modify: `src/quick_launch/mod.rs`
- Modify: `src/quick_launch/index.rs`
- Modify: `src/quick_launch/search.rs`
- Modify: `src/quick_launch/tests/pwsh_test.rs`

**Interfaces:**
- Consumes: `pwsh_profile::ProfileCommand`(Task 2)、`Action::RunPwshCommand`・`split_pwsh_command`(Task 4)
- Produces: `Index.pwsh_functions: Vec<Entry>`、`pub(crate) fn Index::set_pwsh_functions(&mut self, commands: &[ProfileCommand])`、`pub fn Index::search_pwsh_functions(&self, rest: &str) -> Vec<Entry>`

- [ ] **Step 1: 失敗するテストを書く**

`src/quick_launch/tests/pwsh_test.rs` の先頭 `use` の直後へ追加:

```rust
use crate::pwsh_profile::ProfileCommand;

fn index_with_commands(names: &[&str]) -> Index {
    let mut index = Index::default();
    let commands: Vec<ProfileCommand> = names
        .iter()
        .map(|name| ProfileCommand {
            name: (*name).to_string(),
        })
        .collect();
    index.set_pwsh_functions(&commands);
    index
}
```

ファイル末尾へ追加:

```rust
#[test]
fn set_pwsh_functions_builds_entries_with_run_pwsh_command_action() {
    let index = index_with_commands(&["Get-MyThing"]);
    assert_eq!(index.pwsh_functions.len(), 1);
    assert_eq!(index.pwsh_functions[0].name, "Get-MyThing");
    assert_eq!(
        index.pwsh_functions[0].action,
        Action::RunPwshCommand("Get-MyThing".to_string())
    );
}

#[test]
fn search_pwsh_functions_without_args_keeps_bare_name() {
    let index = index_with_commands(&["Get-MyThing", "Get-OtherThing"]);
    let results = index.search_pwsh_functions("Get-My");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Get-MyThing");
    assert_eq!(
        results[0].action,
        Action::RunPwshCommand("Get-MyThing".to_string())
    );
}

#[test]
fn search_pwsh_functions_appends_typed_args_to_label_and_action() {
    let index = index_with_commands(&["Get-MyThing"]);
    let results = index.search_pwsh_functions("Get-MyThing -Foo bar");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Get-MyThing -Foo bar");
    assert_eq!(
        results[0].action,
        Action::RunPwshCommand("Get-MyThing -Foo bar".to_string())
    );
}

#[test]
fn search_pwsh_functions_returns_empty_for_no_match() {
    let index = index_with_commands(&["Get-MyThing"]);
    assert!(index.search_pwsh_functions("Nope").is_empty());
}
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test --lib quick_launch::tests::pwsh_test`
Expected: コンパイルエラー(`Index.pwsh_functions` フィールド、`set_pwsh_functions`、`search_pwsh_functions` が未定義)

- [ ] **Step 3: `Index` にフィールドを追加する**

`src/quick_launch/mod.rs` の `Index` 構造体、`tabs_lower` の直後へ追加:

```rust
    pub(crate) tabs: Vec<Entry>,
    pub(crate) tabs_lower: Vec<search::LowerKeys>,
    /// `pf ` プレフィックス用。PowerShell PROFILE (`$PROFILE`) が定義した
    /// 関数・エイリアス一覧 (FR-9.19)。waypoint 起動時のバックグラウンド
    /// スキャンでのみ更新し、`Index::build` は空で初期化するだけ
    /// (`quick_launch_window::configure` が `set_pwsh_functions` で
    /// 都度再適用する、`tabs` と同じ扱い)。
    pub(crate) pwsh_functions: Vec<Entry>,
```

- [ ] **Step 4: `Index::build` の初期化を直す**

`src/quick_launch/index.rs` の `Index::build` の `Self { ... }` リテラル、`tabs_lower: Vec::new(),` の直後へ追加:

```rust
            tabs: Vec::new(),
            tabs_lower: Vec::new(),
            pwsh_functions: Vec::new(),
```

- [ ] **Step 5: `set_pwsh_functions` を実装する**

`src/quick_launch/index.rs` の先頭 import に追加:

```rust
use crate::pwsh_profile::ProfileCommand;
```

`set_browser_tabs` メソッドの直後(`impl Index` ブロックの `}` の前)へ追加:

```rust
    /// PowerShell PROFILE の関数・エイリアス一覧を差し替える (`pf ` プレフィックス)。
    ///
    /// waypoint 起動時のバックグラウンドスキャン完了通知からのみ呼ばれる。
    /// フル `Index::build` のたびに失われないよう、`set_browser_tabs` と同様に
    /// 呼び出し側 (`quick_launch_window::configure`) が都度再適用する。
    pub(crate) fn set_pwsh_functions(&mut self, commands: &[ProfileCommand]) {
        self.pwsh_functions = commands
            .iter()
            .map(|command| Entry {
                name: command.name.clone(),
                breadcrumb: "PowerShell PROFILE".to_string(),
                path: String::new(),
                action: Action::RunPwshCommand(command.name.clone()),
                branch: None,
            })
            .collect();
    }
```

- [ ] **Step 6: `search_pwsh_functions` を実装する**

`src/quick_launch/search.rs` の `use super::{...}` に `Action` を追加(アルファベット順、先頭へ):

```rust
use super::{
    Action, APPS_PREFIX, AZURE_DEVOPS_PREFIX, BOOKMARK_PREFIX, Entry, HISTORY_PREFIX, Index,
    TABS_PREFIX, TERMINAL_PREFIX, WINDOW_PREFIX,
};
```

`search_cached_work_items` / `merge_cached_work_items` の直後(`impl Index` ブロックの `}` の前)へ追加:

```rust
    /// `pf ` プレフィックス用。最初の半角スペースまでのトークンで関数・
    /// エイリアス名をマッチングし、それより後ろの文字列を実行時引数として
    /// 候補ラベルと `Action::RunPwshCommand` へそのまま連結する。引数は
    /// キー入力のたびに変わるため、事前構築した `Entry` を使い回さず都度
    /// 組み立てて返す (`Index::search` の `Vec<&Entry>` 契約には乗せない、
    /// Everything / Azure ライブ検索と同じ理由)。
    pub fn search_pwsh_functions(&self, rest: &str) -> Vec<Entry> {
        let (name_part, args_part) = super::split_pwsh_command(rest);
        search_entries(&self.pwsh_functions, name_part, false, &self.ranking)
            .into_iter()
            .map(|entry| {
                if args_part.is_empty() {
                    entry.clone()
                } else {
                    let command = format!("{} {}", entry.name, args_part);
                    Entry {
                        name: command.clone(),
                        action: Action::RunPwshCommand(command),
                        ..entry.clone()
                    }
                }
            })
            .collect()
    }
```

- [ ] **Step 7: テストが通ることを確認する**

Run: `cargo test --lib`
Expected: 全テスト PASS(新規 4 テストを含む)

- [ ] **Step 8: clippy 確認**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0

- [ ] **Step 9: Commit**

```bash
git add src/quick_launch/mod.rs src/quick_launch/index.rs src/quick_launch/search.rs \
        src/quick_launch/tests/pwsh_test.rs
git commit -m "$(cat <<'EOF'
feat(quick-launch): Index に PROFILE 関数の保持・検索を追加

set_pwsh_functions で候補一覧を差し替え、search_pwsh_functions で先頭
トークンをマッチングして残りを実行時引数としてラベル・アクションへ連結
する。Quick Launch 側の配線 (quick_launch_window) は次タスクで行う。
EOF
)"
```

---

### Task 6: `quick_launch_window` — `pf ` の検索配線

**Files:**
- Modify: `src/quick_launch_window/mod.rs`
- Modify: `src/quick_launch_window/search.rs`
- Modify: `src/quick_launch_window/tests/search_test.rs`

**Interfaces:**
- Consumes: `Index::set_pwsh_functions` / `Index::search_pwsh_functions`(Task 5)、`PWSH_PROFILE_PREFIX`(Task 4)、`QuickLaunchSettings.include_pwsh_functions`(Task 3)、`pwsh_profile::ProfileCommand`(Task 2)
- Produces: `pub fn quick_launch_window::set_pwsh_functions(commands: Vec<ProfileCommand>)`(Task 7 が呼ぶ)

- [ ] **Step 1: 失敗するテストを書く**

`src/quick_launch_window/tests/search_test.rs` の `refined_search_only_reuses_candidates_for_a_narrower_local_query` テスト内、`az pr`/`az pra` のアサーションの直後へ追加:

```rust
    assert_eq!(refinable_search_term(Some("az pr"), "az pra", 1), None);
    assert_eq!(
        refinable_search_term(Some("pf Get"), "pf Get-Foo", 1),
        None
    );
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test --lib quick_launch_window::tests::search_test`
Expected: FAIL(`local_search_scope` が `"pf "` を通常検索と同じ `"normal"` 扱いにしてしまい、`refinable_search_term` が `None` ではなく `Some(...)` を返す)

- [ ] **Step 3: `local_search_scope` から `pf ` を除外する**

`src/quick_launch_window/search.rs` の `local_search_scope` 冒頭のガードに追加:

```rust
fn local_search_scope(query: &str) -> Option<(&'static str, &str)> {
    if query.starts_with(crate::quick_launch::EVERYTHING_PREFIX)
        || query.starts_with(crate::quick_launch::AZURE_DEVOPS_PREFIX)
        || query.starts_with(crate::quick_launch::PWSH_PROFILE_PREFIX)
    {
        return None;
    }
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test --lib quick_launch_window::tests::search_test`
Expected: PASS

- [ ] **Step 5: `State` にフィールドを追加する**

`src/quick_launch_window/mod.rs` の `State` 構造体、`browser_tabs` の直後へ追加:

```rust
    /// 拡張から受け取った現在のブラウザタブ。Index を再構築しても失わないよう、
    /// 検索インデックスとは別にメモリ上で保持する。
    browser_tabs: Vec<(crate::browser_tabs::Browser, crate::browser_tabs::Tab)>,
    /// waypoint 起動時のバックグラウンドスキャンで取得した PROFILE の
    /// 関数・エイリアス一覧。`browser_tabs` と同じ理由で Index とは別に
    /// メモリ上に保持し、`Index::build` のたびに `set_pwsh_functions` で
    /// 再適用する。
    pwsh_functions: Vec<crate::pwsh_profile::ProfileCommand>,
```

`everything_enabled` の直後へ追加:

```rust
    everything_enabled: bool,
    /// `include_pwsh_functions` 設定。`pf ` モード中にこれが偽なら検索せず
    /// 「無効化されています」の案内だけを出す (`everything_enabled` と同じ扱い)。
    pwsh_enabled: bool,
```

- [ ] **Step 6: `configure` / `configure_dynamic` で再適用する**

`configure` 内、`state.index.set_browser_tabs(&tabs);` の直後へ追加:

```rust
            state.index.set_browser_tabs(&tabs);
            let pwsh_functions = state.pwsh_functions.clone();
            state.index.set_pwsh_functions(&pwsh_functions);
            state.pwsh_enabled = config.settings.quick_launch.include_pwsh_functions;
```

`configure_dynamic` 内、`state.index.set_browser_tabs(&tabs);` の直後へ追加:

```rust
            state.index.set_browser_tabs(&tabs);
            let pwsh_functions = state.pwsh_functions.clone();
            state.index.set_pwsh_functions(&pwsh_functions);
```

- [ ] **Step 7: バックグラウンドスキャン結果を受け取るセッターを追加する**

`replace_browser_tabs` 関数の直後へ追加:

```rust
/// バックグラウンドスキャン (`pwsh_profile::scan_async`) が届けた PROFILE の
/// 関数・エイリアス一覧を差し替える。Quick Launch が表示中なら `pf ` の
/// 検索結果も即座に反映する。
pub fn set_pwsh_functions(commands: Vec<crate::pwsh_profile::ProfileCommand>) {
    STATE.with(|state| {
        let has_window = {
            let mut state = state.borrow_mut();
            state.pwsh_functions = commands;
            let pwsh_functions = state.pwsh_functions.clone();
            state.index.set_pwsh_functions(&pwsh_functions);
            state.previous_query = None;
            state.window.is_some()
        };
        if has_window {
            update_results(state);
        }
    });
}
```

- [ ] **Step 8: `update_results` に `pf ` の分岐を追加する**

`src/quick_launch_window/search.rs` の `update_results` 内、Everything の分岐の直後へ追加:

```rust
    if let Some(rest) = query.strip_prefix(crate::quick_launch::EVERYTHING_PREFIX) {
        start_everything_query(state, rest);
        return;
    }
    if let Some(rest) = query.strip_prefix(crate::quick_launch::PWSH_PROFILE_PREFIX) {
        start_pwsh_query(state, rest);
        return;
    }
```

`start_everything_query` の直後へ関数を追加:

```rust
/// `pf ` に入った。関数・エイリアス一覧はメモリ上にキャッシュ済みなので、
/// Everything / Azure ライブ検索と違い非同期の応答待ちを挟まず即座に
/// 結果を反映する (`Index::search_pwsh_functions` 参照)。
pub(super) fn start_pwsh_query(state: &RefCell<State>, rest: &str) {
    let (list, has_results, message) = {
        let mut state = state.borrow_mut();
        state.everything_active = false;
        state.azure_work_items_active = false;
        state.previous_query = None;
        state.highlight_term.clear();
        if state.pwsh_enabled {
            state.results = state
                .index
                .search_pwsh_functions(rest)
                .into_iter()
                .take(MAX_LIST_RESULTS)
                .collect();
            state.empty_message = None;
        } else {
            state.results.clear();
            state.empty_message =
                Some("PowerShell PROFILE search is disabled in Settings.".to_string());
        }
        (
            state.list,
            !state.results.is_empty(),
            state.empty_message.clone(),
        )
    };
    let Some(list) = list else {
        return;
    };
    let (labels, rows) = {
        let mut state = state.borrow_mut();
        let (labels, rows) = build_rows(&state.results, &[]);
        state.rows = if rows.is_empty() {
            vec![RowKind::Message]
        } else {
            rows.clone()
        };
        (labels, rows)
    };
    if has_results {
        populate_list(list, &labels, &rows);
    } else {
        populate_empty_message(list, message.as_deref());
    }
}
```

- [ ] **Step 9: ビルドとテストを確認する**

Run: `cargo build && cargo test --lib`
Expected: 全テスト PASS

- [ ] **Step 10: clippy 確認**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0

- [ ] **Step 11: Commit**

```bash
git add src/quick_launch_window/mod.rs src/quick_launch_window/search.rs \
        src/quick_launch_window/tests/search_test.rs
git commit -m "$(cat <<'EOF'
feat(quick-launch-window): pf の検索配線を追加

State に pwsh_functions/pwsh_enabled を持たせ、Index 再構築のたびに
再適用する (tabs と同じパターン)。update_results に pf の同期検索分岐を
追加し、無効化設定時は案内メッセージを出す。バックグラウンドスキャンの
結果を受け取る set_pwsh_functions はまだどこからも呼ばれない
(tray/main.rs への配線は次タスク)。
EOF
)"
```

---

### Task 7: 起動時スキャンの配線 (`tray` / `main.rs`)

**Files:**
- Modify: `src/tray/mod.rs`
- Modify: `src/tray/window.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `pwsh_profile::scan_async` / `take_scanned`(Task 2)、`quick_launch_window::set_pwsh_functions`(Task 6)

- [ ] **Step 1: メッセージ定数を追加する**

`src/tray/mod.rs` の `WM_DYNAMIC_REFRESHED` の直後へ追加:

```rust
pub const WM_DYNAMIC_REFRESHED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;
/// PowerShell PROFILE の関数・エイリアス一覧のバックグラウンドスキャンが
/// 完了した通知 (`pwsh_profile::scan_async`)。
pub const WM_PWSH_PROFILE_SCANNED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 9;
```

- [ ] **Step 2: スキャン開始関数を追加する**

`refresh_azure_devops` の直後へ追加:

```rust
/// PowerShell PROFILE の関数・エイリアス一覧をバックグラウンドで取得する。
/// waypoint 起動時に一度だけ呼ぶ (`main.rs`)。設定で無効化されていれば
/// 何もしない。
pub fn start_pwsh_profile_scan(hwnd: HWND) {
    STATE.with(|state| {
        let state = state.borrow();
        let Some(state) = state.as_ref() else {
            return;
        };
        if state.config.settings.quick_launch.include_pwsh_functions {
            crate::pwsh_profile::scan_async(hwnd, WM_PWSH_PROFILE_SCANNED);
        }
    });
}
```

- [ ] **Step 3: メッセージハンドラを追加する**

`src/tray/window.rs` の `use super::{...}` に `WM_PWSH_PROFILE_SCANNED` を追加:

```rust
use super::{
    AZURE_FULL_REFRESH_TIMER_ID, WM_AZURE_DEVOPS_REFRESHED, WM_DYNAMIC_REFRESHED,
    WM_PWSH_PROFILE_SCANNED, WM_RELOAD_CONFIG, WM_TRAY, refresh_azure_devops, reload, with_state,
};
```

`WM_DYNAMIC_REFRESHED` のハンドラの直後へ追加:

```rust
        WM_DYNAMIC_REFRESHED => {
            handle_dynamic_refreshed();
            LRESULT(0)
        }
        WM_PWSH_PROFILE_SCANNED => {
            quick_launch_window::set_pwsh_functions(crate::pwsh_profile::take_scanned());
            LRESULT(0)
        }
```

- [ ] **Step 4: 起動時に呼ぶ**

`src/main.rs` の `tray::start_azure_full_refresh_timer(hwnd);` の直後へ追加:

```rust
    tray::refresh_azure_devops(hwnd);
    tray::start_azure_full_refresh_timer(hwnd);
    tray::start_pwsh_profile_scan(hwnd);
```

- [ ] **Step 5: ビルドとテストを確認する**

Run: `cargo build && cargo test --lib`
Expected: 全テスト PASS

- [ ] **Step 6: clippy 確認**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0

- [ ] **Step 7: Commit**

```bash
git add src/tray/mod.rs src/tray/window.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(tray): waypoint 起動時に PROFILE 関数のバックグラウンドスキャンを開始

WM_PWSH_PROFILE_SCANNED でスキャン完了を受け取り、Quick Launch の
pwsh_functions キャッシュへ反映する。これで pf プレフィックスが実際に
機能するようになる。
EOF
)"
```

---

### Task 8: 最終検証

**Files:** なし(検証のみ)

- [ ] **Step 1: 実行中の waypoint を終了する**

Run: `Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force`

- [ ] **Step 2: フルビルド**

Run: `cargo build`
Expected: 成功

- [ ] **Step 3: 全テスト**

Run: `cargo test`
Expected: 全 PASS

- [ ] **Step 4: clippy(CI と同じ検査)**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 警告 0

- [ ] **Step 5: フォーマット**

Run: `cargo fmt --all -- --check`
Expected: 差分なし(差分があれば `cargo fmt --all` してから再コミット)

- [ ] **Step 6: 自己診断**

Run:
```powershell
cargo run -- --selftest
Get-Content "$env:TEMP\waypoint_selftest.txt"
```
Expected: `PASS` で始まる出力

- [ ] **Step 7: 手動確認(GUI)**

1. `cargo run` で常駐起動する
2. Quick Launch のホットキー(既定 `Alt+Space`)で検索窓を開く
3. `pf ` と入力し、`PWSH` バッジと `$PROFILE` で定義した関数・エイリアスの一覧が出ることを確認する(自分の `$PROFILE` に関数が無ければ 0 件表示になるのが正しい — その場合は `$PROFILE` に `function Test-Waypoint { param($msg) "hello $msg" }` のようなテスト用関数を一時的に追加してから waypoint を再起動して確認する)
4. 関数名を選び Enter で実行し、Windows Terminal + PowerShell 7 が開いて実行結果が表示されることを確認する
5. `pf <関数名> <引数>` の形で入力し、候補ラベルが `関数名 引数` の形で更新されること、Enter で引数付きに実行されることを確認する
6. 設定画面(トレイ → Settings)の Quick Launch タブで "Run PowerShell PROFILE functions" のチェックを外して保存し、`pf ` が「disabled」の案内になることを確認する

- [ ] **Step 8: 完了報告**

上記すべてが確認できたら、このタスクへの追加コミットは不要(検証のみのため)。手動確認 (Step 7) の結果をユーザーへ報告する。
