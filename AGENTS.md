# AGENTS.md

Guidance for AI coding agents working in this repository (read by Claude Code via CLAUDE.md import, and by other agents such as Codex / Copilot / Cursor directly).

## What this is

waypoint is a Windows tray-resident popup launcher (Rust). A hotkey or the middle mouse button opens a hierarchical popup menu; selecting an entry jumps to that location in Explorer. It is a re-implementation of Quick Access Popup's core, narrowed to folder navigation.

Single developer, not distributed. No installer, no auto-update, no localization.

UI text is **English**. Code comments are **Japanese** — keep both conventions when editing.

Full specification: `docs/spec.md`.

## Product priorities

Weigh every change against these, in order:

1. **Display latency** — trigger to painted menu within 50ms. This is the whole point of the app; a slow menu is a failed menu. Don't add work to the trigger path.
2. **Don't grow the scope** — see the scope rule below.
3. **Keyboard-complete operation** — every frequent action reachable without the mouse.

## Commands

```powershell
cargo build                                  # ビルド
cargo test                                   # テスト
cargo clippy --all-targets -- -D warnings    # CI と同じ静的検査
cargo fmt --all                              # 整形
cargo run                                    # 起動(トレイ常駐)

# GUI サブシステムのため stdout に出せない。自己診断は結果をファイルに書く
cargo run -- --selftest
Get-Content "$env:TEMP\waypoint_selftest.txt"
```

- **リビルド前に実行中の `waypoint` を必ず終了する。** 実行中の exe は出力をロックし、ビルドが「アクセスが拒否されました (os error 5)」で落ちる。ビルドエラーに見えるが原因は別。
  ```powershell
  Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force
  ```
- **警告 0 を維持する。** CI が `-D warnings` で強制する。

## The scope rule

`docs/spec.md` の 02 章に、機能ごとの「採用 / 保留 / 除外」判定表がある。**「除外」の項目は v1.0 では実装しない。** 実装中に思いついても着手しない。

追加したくなった場合は、**先に `docs/spec.md` の表を書き換えてから**着手する。仕様書を更新せずに機能を足すことを禁止する。この規約が仕様書の存在意義。

## Known pitfalls

実装前に把握しておくべき落とし穴。上ほど早期に踏む。

### 低レベルフックはタイムアウトすると黙って外される
`WH_MOUSE_LL` のコールバック応答がレジストリの `LowLevelHooksTimeout`（既定 300ms）を超えると、Windows はフックを**通知なく解除する**。以後トリガーが効かなくなり、再現しづらい不具合になる。

→ **フック内では判定のみ行う。** メニュー構築と表示は `PostMessage` した先で実行する。

### TrackPopupMenuEx の前に SetForegroundWindow が要る
呼び出し元ウィンドウが最前面でないと、メニュー外をクリックしてもメニューが閉じない。Win32 の古典的なハマりどころ。`src/tray.rs` の `track_menu()` が既にこの順序で書いてある。

### 元ウィンドウのハンドルはトリガー時点で保持する
メニューを表示した瞬間に自分が最前面になるため、「元のエクスプローラーのフォルダを変更する」(`reuse`) の対象を見失う。トリガー検出の時点で `GetForegroundWindow` の結果を保存しておくこと。

### 中ボタンの横取りは他アプリを壊す
無条件に奪うと、ブラウザのオートスクロールとリンクの新規タブオープンが機能しなくなる。**除外プロセス一覧 (FR-1.4) とドラッグ判定 (FR-1.5) は最初の実装に含める。** 後付けにすると設計の作り直しになる。

### その他
- `TrackPopupMenuEx` の座標は**物理ピクセル**。Per-Monitor V2 を宣言済み (`main.rs`)、DPI 変換は 1 箇所に閉じ込める。
- 管理者権限で動くアプリのウィンドウにはフックもメッセージも届かない (UIPI)。仕様として受け入れ、対応しない。
- ネットワークパスの存在確認はメニュー表示を固めうる。非同期にし、結果が返るまでは通常表示。
- COM (エクスプローラー操作) は `CoInitializeEx(COINIT_APARTMENTTHREADED)` が必要。UI スレッドから呼ぶ。
- フックプロシージャは `unsafe extern "system"`。`LPARAM` → `MSLLHOOKSTRUCT` は生ポインタ参照になるので `unsafe` の範囲を最小に閉じる。

## Reference implementations

同じ Win32 呼び出しの実例が手元にある。**読んで作法を踏襲する**（コピーではない）。

| 必要なもの | 参照先 |
|---|---|
| `SHGetKnownFolderPath` の呼び方・`PWSTR` 解放 | `E:\Windows-ComputerUse\src\win.rs:38` |
| `GetForegroundWindow` | `E:\Windows-ComputerUse\src\window.rs:221` |
| `SetForegroundWindow` + `AllowSetForegroundWindow` | `E:\Windows-ComputerUse\src\window.rs:292` |
| `windows` crate の features 指定 | `E:\Windows-ComputerUse\Cargo.toml` |

設計の参考（言語は違うが同種のトレイ常駐アプリ）:

| 設計 | 参照先 |
|---|---|
| 設定の原子的保存 (temp → replace) | `E:\rdp-manager\src\RdpManager\Services\AtomicWrite.cs` |
| パース失敗時に `.bak` 退避してから再シード | `E:\rdp-manager\src\RdpManager\Services\ConnectionStore.cs` |
| 自己診断モード (`--selftest`) | `E:\rdp-manager\AGENTS.md` |

## Current state

トレイに常駐して終了できるところまで。**ポップアップ表示・フック・ホットキー・設定読み込みはすべて未実装。**

次にやること: 仕様書 P0 の技術検証（`TrackPopupMenuEx` の実表示 / `WH_MOUSE_LL` の実フック / `RegisterHotKey` / エクスプローラー操作）。型が通ることは確認済みなので、残るは実挙動の確認。

`src/` のモジュール分割 (`menu` / `hook` / `config` / `shell`) は P0 の結果を見てから切る。FR-6 管理画面の GUI 方式（`native-windows-gui` / `egui` / Win32 直書き）は常駐部が動いてから決める。**今は GUI クレートを入れていない。**
