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

# MSI をビルドする (WiX 5 が必要)
.\installer\build.ps1
.\installer\build.ps1 -SkipBuild   # ビルド済みバイナリを使う
```

- **リビルド前に実行中の `waypoint` を必ず終了する。** 実行中の exe は出力をロックし、ビルドが「アクセスが拒否されました (os error 5)」で落ちる。ビルドエラーに見えるが原因は別。
  ```powershell
  Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force
  ```
- **警告 0 を維持する。** CI が `-D warnings` で強制する。

## リリース手順

1. `Cargo.toml` の `version` を上げる。**バージョンの出どころはここだけ**
   （`installer/build.ps1` が読んで `-d Version=` で wxs へ渡す）
2. `.\installer\build.ps1` で `dist\waypoint-<version>-x64.msi` を作る
3. `git tag vX.Y.Z && git push --tags`
   → `installer.yml` が Release に MSI を添付する

インストーラの決定事項は `docs/spec.md` の「08-2. 配布形態」にある。

- **`UpgradeCode` は変えない。** 変えると別製品扱いになり、古い版が残る
- MSI は **perUser**。`ProgramFiles` は使えないので `LocalAppDataFolder` へ入れる
- **`util:CloseApplication` を消さない。** 常駐中の exe を掴んだままだと
  上書きに失敗し「使用中のファイル」ダイアログや再起動要求になる (R-10)
- 説明文に日本語を使うので `Codepage="65001"` が要る。既定の 1252 では
  `WIX0311` で通らない
- ARP の登録先は perUser でも **`HKLM\...\Uninstall`** と
  `Installer\UserData\<SID>\Products`。`HKCU\...\Uninstall` を見ても無い

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

### GetLastError は Win32 呼び出しの「直後」に読む
`CreateMutexW` の直後は 183 (`ERROR_ALREADY_EXISTS`) だが、`Result` を分解するだけで
別の Win32 呼び出しが挟まり値が上書きされる。実測:

```
second lasterr(immediately) = 183  already=true
second lasterr(after match) = 6    already=false
```

これで二重起動抑止が動いていなかった (`src/single.rs`) 。同じ罠は
`RegisterClassW` / `CreateWindowExW` の戻り値判定にもある。

### 名前付き Mutex の所有権を取らない
`CreateMutexW(None, true, ...)` で所有権を握ると、強制終了されたプロセスの mutex が
abandoned 状態で残り、次の起動が待たされて無応答になる。存在確認しかしないので
第 2 引数は `false`。名前空間も単一ユーザー向けなので `Local\` で足りる。

### 既定ホットキーに Win+W は使えない
Windows 11 では Widgets が予約済みで `RegisterHotKey` が「既に登録されています」で
失敗する (`Win+Q` も検索が予約) 。実測で確認済み。既定は `Ctrl+Alt+W`。
ホットキーが取れなかった場合はトレイメニューに警告を出す (無言で効かないと原因が分からない) 。

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

**動く。** 中ボタン / ホットキー / トレイ左クリックでメニューが出て、選んだフォルダが
Windows の既定フォルダーハンドラーで開く。実機で確認済み。

実装済み:
- FR-1 トリガー (中ボタン・ホットキー・トレイ左クリック・除外プロセス・ドラッグ判定)
- FR-2 メニュー表示 (階層・アイコン・数字アクセラレータ・グレー表示)
- FR-3 4 種類の項目、FR-4 newWindow / reuse、FR-5 変数展開
- FR-6 管理画面 (項目・ユーザー変数・トリガー設定の編集、保存後の即時反映)
- FR-7 設定の読み書き (原子的保存・パース失敗時の継続起動)
- FR-8 常駐 / トレイメニュー / 二重起動抑止 / 自動起動

### FR-6 管理画面の構成

`waypoint-settings.exe` を Rust + `egui/eframe` の別プロセスとして実装している。
トレイメニューの "Settings..." から必要時のみ起動し、保存後は `PostMessage` で
常駐部へ再読み込みを通知する。常駐部の起動時間・メモリ・メッセージループには
GUI フレームワークを持ち込まない。

`docs/settings_ui.rs.wip` は `native-windows-gui` で試作した旧実装として残してある。
同方式はリンク後の通常起動で次のローダエラーになるため採用しない:

```
プロシージャ エントリ ポイント GetWindowSubclass が
ダイナミック リンク ライブラリ waypoint.exe から見つかりませんでした
```

**マニフェスト自体は残してある** — Per-Monitor V2 / longPathAware /
Visual Styles はこのアプリに必要で、GUI とは独立に効く。
