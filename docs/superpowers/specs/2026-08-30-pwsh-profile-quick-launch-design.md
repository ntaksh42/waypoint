# Quick Launch から PowerShell PROFILE の関数・エイリアスを実行する (`pf `)

## 概要

Quick Launch に新しい検索プレフィックス `pf ` を追加する。PowerShell 7
(`pwsh`) の `$PROFILE` が定義した関数・エイリアスだけを検索対象とし、
選択すると引数付きで Windows Terminal 上に即実行する。

対応する仕様書の変更は `docs/spec.md` 02章の表（採用行を追加）と
FR-9.19（このドキュメントの完成版）。

## スコープ

- 対象は PowerShell 7 (`pwsh`) の `$PROFILE`。Windows PowerShell 5.1
  (`powershell.exe`) の PROFILE は対象外
- 対象は「PROFILE 自体が定義した」関数・エイリアスのみ。`Import-Module`
  で読み込まれるモジュール（PSReadLine, posh-git, Terminal-Icons 等）が
  公開するコマンドと、素の PowerShell エンジンが内蔵するエイリアス
  （`ls` / `cd` / `gci` 等）は対象外
- `pwsh` が見つからない環境では `pf ` は常に 0 件（エラー表示なし）。
  Windows PowerShell 5.1 へのフォールバックはしない

## データ取得: 差分診断による一覧化

新規モジュール `src/pwsh_profile.rs`（`apps.rs` と同じ立ち位置、Win32
に触らない純ロジック）が、waypoint 起動時にバックグラウンドスレッドで
一度だけ以下を行う。

1. `pwsh -NoProfile -Command "Get-Command -CommandType Function,Alias | Select-Object -ExpandProperty Name"`
   でベースライン（エンジン組み込みのみ）を取得する
2. `pwsh -Command "Get-Command -CommandType Function,Alias | Select-Object -ExpandProperty Name"`
   で通常起動（全 PROFILE スコープ読み込み済み）の一覧を取得する
3. (2) から (1) を除いた差分が「PROFILE が定義したもの」。名前の
   大文字小文字を無視して重複排除し、ソートする

2 回起動する分コストは重い（環境依存で数百 ms 〜 数秒）が、起動時の
非同期バックグラウンドでのみ実行し、設定保存や動的メニュー更新の
たびには再実行しない（利用者の明示的な再取得トリガーのみ再実行、
FR-9.19 参照）。`pwsh` の実行ファイルは `shell.rs::find_pwsh()` を
そのまま流用して解決する。

```rust
// src/pwsh_profile.rs (概形)
pub struct ProfileCommand {
    pub name: String,
}

pub fn scan() -> Vec<ProfileCommand> { /* 2 回起動 + 差分 */ }
```

## 統合: Index / STATE への組み込み

`browser_tabs`（`t ` プレフィックス）と同じパターンを踏襲する。

- `quick_launch_window::STATE` に `pwsh_functions: Vec<ProfileCommand>`
  を追加する。起動時に spawn したスレッドが `pwsh_profile::scan()` を
  実行し、完了後に新規メッセージ `WM_PWSH_PROFILE_SCANNED`
  (`WM_APP + 11`、`tray/mod.rs` に定義) で結果を渡す
- `Index` に `set_pwsh_functions(&mut self, commands: &[ProfileCommand])`
  を追加し、`set_browser_tabs` と同様に `configure` /
  `configure_dynamic` / `refresh_config_items` / `refresh_azure` の
  呼び出し後に再適用して、フル `Index::build` のたびに失われないよう
  にする
- `Index::build` 自体は `pwsh_functions` を空で初期化するだけで、
  pwsh を起動しない（起動コストをビルド経路に持ち込まない）

## 検索: `Index::search` の同期モデルには乗せない

Everything / Azure ライブ検索と同じ理由（`mod.rs` の
`EVERYTHING_PREFIX` 解説を参照）で、`pf ` は `Index::search` の
`Vec<&Entry>` 契約には乗らない。候補ラベルが打鍵中の引数テキストに
依存し、事前構築した `Entry` を使い回せないため、専用メソッドを
新設し window 層（`quick_launch_window/dispatch.rs` 相当）から直接
呼ぶ。

```rust
impl Index {
    /// `pf ` モード用。`name_part` で関数・エイリアス名をマッチングし、
    /// `args_part` を候補ラベルと実行コマンドへそのまま連結する。
    pub fn search_pwsh_functions(&self, name_part: &str, args_part: &str) -> Vec<Entry> {
        // self.pwsh_functions を通常のランキング (FR-9.5 相当) でマッチ
        // 上位候補ごとに Entry { name: "<Name> <args_part>".trim(), ... } を組み立てる
    }
}
```

- クエリを最初の半角スペースで `name_part` / `args_part` に分割する
  （`"pf Get-MyThing -Foo bar"` → `name_part = "Get-MyThing"`,
  `args_part = "-Foo bar"`）
- `args_part` が空なら候補ラベルは関数名のみ、非空なら
  `"<Name> <args_part>"` を毎キー入力ごとに組み立てて表示する
- 候補数十〜百件規模のメモリ内マッチングなので、キー入力経路の
  コストは他のプレフィックス検索と同等（サブ ms 級）

## 実行

新規 `Action::RunPwshCommand(String)`（実行するコマンド全文、例:
`"Get-MyThing -Foo bar"`）を追加する。

`shell.rs` に `run_pwsh_command(command: &str)` を追加し、既存の
`open_terminal` と同じ `wt.exe <pwsh のフルパス> -NoExit -Command
<command>` パターンで起動する（`wt.exe` の PATH 解決の癖に対応する
ため `pwsh.exe` のフルパスを渡す既存の処理を流用）。`args_part` は
利用者が入力した PowerShell コード片としてそのままコマンドライン
引数に渡るだけで、waypoint 側でのエスケープ処理は行わない（Run
ダイアログ相当の挙動であり、利用者自身が打った内容を利用者自身の
PROFILE 上で実行するだけなのでインジェクションの懸念はない）。

`quick_launch_history.rs::key_kind` では `RunPwshCommand` を `None`
とし、Recent/Frequent の使用履歴には記録しない（`FocusBrowserTab` と
同じ扱い。引数を含め毎回組み合わせが変わりうるため、名前だけの
ランキングは意味を持たない）。

## 設定・UI

- `QuickLaunchSettings` に `include_pwsh_functions: bool`（既定
  `true`）を追加
- FR-6 の管理画面にトグルを追加
- `mod.rs::prefix_badge` に `"pf "` → `"PWSH"` を追加
- `mod.rs::TERMINAL_PREFIX` 等と同じ場所に `PWSH_PROFILE_PREFIX: &str
  = "pf "` を追加

## テスト方針

- `pwsh_profile::scan()` の差分ロジック（ベースライン集合と通常起動
  集合の差分、大文字小文字を無視した重複排除）はユニットテスト化する
  （`Get-Command` の実行結果を模した `Vec<String>` を入力に取る内部
  関数に切り出す）
- `pwsh` が見つからない場合に空を返すことをテストする
- `Index::search_pwsh_functions` の分割・マッチング・ラベル組み立てを
  `quick_launch/tests/` にユニットテスト追加（既存の `search_test.rs`
  のパターンに倣う）
- 実際の pwsh 起動・ターミナル実行は統合テストの対象外（既存の
  `open_terminal` も同様、手動確認）

## 影響ファイル一覧

| ファイル | 変更内容 |
|---|---|
| `src/pwsh_profile.rs` (新規) | 差分診断ロジック |
| `src/shell.rs` | `run_pwsh_command` 追加 |
| `src/quick_launch/mod.rs` | プレフィックス定数・badge・`Action::RunPwshCommand` 追加 |
| `src/quick_launch/index.rs` | `pwsh_functions` フィールド・`set_pwsh_functions`・`search_pwsh_functions` 追加 |
| `src/quick_launch_window/mod.rs` | `STATE.pwsh_functions`、`WM_PWSH_PROFILE_SCANNED` ハンドリング、起動時スレッド起動 |
| `src/quick_launch_window/dispatch.rs` | `pf ` モードの分岐・実行ディスパッチ |
| `src/tray/mod.rs` | `WM_PWSH_PROFILE_SCANNED` 定数追加 |
| `src/quick_launch_history.rs` | `key_kind` に `RunPwshCommand => None` 追加 |
| `src/config/*` | `include_pwsh_functions` 設定項目追加 |
| `src/settings_main/*` | 管理画面のトグル追加 |
| `docs/spec.md` | 02章の表、FR-9.19（済み） |
