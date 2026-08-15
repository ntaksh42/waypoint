# AGENTS.md

Guidance for AI coding agents working in this repository (read by Claude Code via CLAUDE.md import, and by other agents such as Codex / Copilot / Cursor directly).

## What this is

waypoint is a Windows tray-resident popup launcher (Rust). A hotkey or the middle mouse button opens a hierarchical popup menu; selecting an entry jumps to a folder, focuses a window, opens a bookmark, or launches an app. It started as a re-implementation of Quick Access Popup's core narrowed to folder navigation, and has since grown into a general launcher — Quick Launch's search box now covers folders, open windows, browser bookmarks (`b ` prefix), and installed apps (`a ` prefix). See `docs/spec.md` 02 for what's in scope and why.

Single developer, not distributed. No installer, no auto-update, no localization.

UI text is **English**. Code comments are **Japanese** — keep both conventions when editing.

Full specification: `docs/spec.md`.

## Product priorities

The goal is to be the most capable general launcher on Windows — broader than Flow Launcher / PowerToys Run in what it can search and do, without becoming slower or harder to use than either. Weigh every change against these, in order:

1. **Display latency** — trigger to painted menu within 50ms. This is non-negotiable regardless of how many features exist; a slow menu is a failed menu. Don't add work to the trigger path. Heavy or optional features (external IPC, network calls, big indexes) must be async/opt-in/lazy so their cost never lands on the hot path — see Everything (`f ` prefix) in `src/everything.rs` for the pattern.
2. **Follow the scope table, not scope minimalism** — see the scope rule below. Breadth of capability is now a stated goal, not something to resist; the table exists so additions are deliberate and documented, not to keep the feature set small.
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

`docs/spec.md` の 02 章に、機能ごとの「採用 / 保留 / 除外」判定表がある。目標が「最も高機能な汎用ランチャー」になったことで採用の対象は大きく広がったが、**「除外」の項目は実装しない。** 除外は「機能が多すぎるから切った」のではなく、waypoint の設計原則（50ms のトリガー応答、単一ユーザー・非配布、キーストローク送出系のミニ言語を持たない、等）と衝突するために外したもの。実装中に思いついても着手しない。

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

### Win+W は環境によって RegisterHotKey が失敗する
Windows 11 では Widgets が予約済みで `RegisterHotKey` が「既に登録されています」で
失敗する環境がある (`Win+Q` も検索が予約) 。実測で確認済み。

**失敗した場合は `WH_KEYBOARD_LL` で横取りする** (FR-1.2.1、`src/trigger.rs`) 。
低レベルフックは OS のホットキー処理より手前で打鍵を受け取るため、既に他が
握っているキーでも上書きできる。`--selftest` の `hotkey="..":native|hook|failed`
でどちらの経路か分かる。`failed` はキー指定が不正な場合だけで、そのときは
トレイメニューに警告を出す (無言で効かないと原因が分からない) 。

フックで横取りするときの注意:

- **Win 修飾の打鍵を握り潰すとスタートメニューが開く。** シェルが「Win 単独押下」
  と解釈するため。無害な Ctrl の打鍵を `SendInput` で挟んで連鎖を切る
- **合成入力を `LLKHF_INJECTED` で一律に外さない。** 自分が注入した Ctrl は
  `dwExtraInfo` の目印 (`OWN_INPUT_TAG`) で見分ける。フラグで外すと
  キーリマッパーや自動化ツール経由の打鍵まで拾えなくなる
- **押下を消費したら対になる解放も消費する。** 解放だけがアプリへ届く状態を作らない
- 同じキーをフックで奪い合う常駐アプリ (QAP など) が居る場合、**後から
  フックを張った側が勝つ**。実測で waypoint が QAP より優先されることを確認済み

### メニューの行高はアイコンサイズでは変わらない
`MIIM_BITMAP` で 32px のアイコンを付けても、`TrackPopupMenuEx` の行高は
システムの `SM_CYMENU` のまま。**アイコンだけが大きくなり行は詰まったまま**になる。
実測で確認済み。

行の高さを制御する手段はオーナードローしかない (`src/menu_draw.rs`) 。
`MFT_OWNERDRAW` で追加し、`WM_MEASUREITEM` で高さを返し `WM_DRAWITEM` で描く。
フォントは `SPI_GETNONCLIENTMETRICS` の `lfMenuFont` をそのまま使うので
文字サイズは変わらない (QAP も同じ方針で、メニューフォントは変更できない) 。

- **`itemData` は `MIIM_DATA` でしか渡せない。** `AppendMenuW` の `lpnewitem` に
  入れても文字列として解釈される。`InsertMenuItemW` + `MENUITEMINFOW` を使う
- **オーナードローでは `&` を自分で解釈しない。** `&1` のアクセラレータ装飾は
  描画前に落とす (`strip_accelerator`) 。`&&` はリテラルの `&` へ戻す

### オーナードローにするとダークメニューが白くなる
`theme::enable_dark_menus()` は uxtheme にダーク描画を指示するもので、
**`GetSysColor` の戻り値は変えない**。オーナードローで背景と文字色を
`COLOR_MENU` / `COLOR_MENUTEXT` から取ると、ライトの色で塗ってしまい
FR-2.7 に反する (実測でメニューが白背景になった) 。

色は uxtheme のメニューテーマから引く:
`OpenThemeData(None, w!("Menu"))` → `DrawThemeBackground` (背景) /
`GetThemeColor(..., TMT_TEXTCOLOR)` (文字色) 。ダーク指定がそのまま乗る。

- `MENU_POPUPBACKGROUND` で地を敷いてから `MENU_POPUPITEM` を状態付きで重ねる
- 状態は `MPI_NORMAL` / `MPI_HOT` / `MPI_DISABLED` を項目の状態に対応させる
- **`DrawFrameControl` はシステム色固定。** サブメニュー矢印に使うと
  ダークで沈んで見えない。設定済みの文字色で `▶` を描く
- **`MF_SEPARATOR` も残さない。** 項目だけオーナードローにすると、
  区切り線は Windows がシステム色で描き**白い線として浮く**(実測)。
  `MFT_OWNERDRAW | MFT_SEPARATOR` にして自前で引く
- **通常項目の背景色はテーマから取れない。** 実測で
  `MENU_POPUPBACKGROUND` と `MPI_NORMAL` の `TMT_FILLCOLOR` は
  `0x80070490` (要素なし) を返し、`MPI_NORMAL` の項目自体も透明。
  `DrawThemeBackground` に任せると明るい地が出る。地は自前で塗り、
  ダーク判定は `MPI_NORMAL` の**文字色**が明るいか (白=ダーク) で行う
- テーマハンドルは掴んだままにせず、`WM_THEMECHANGED` で捨てて開き直す

### ポップアップの外枠は暗くできない (未解決・対応しない)

項目と区切り線をダークにしても、**ポップアップを囲む 1 px の外枠だけは
明るいまま残る**。ここは非クライアント領域でオーナードローが届かない。
三手試して全て駄目だったので、**v1.0 では受け入れる**。再挑戦するなら
以下は試済みなので繰り返さないこと:

| 試した手 | 結果 |
|---|---|
| `DWMWA_BORDER_COLOR` を設定 | `Ok(())` が返るのに色が変わらない。メニュー (`#32768`) は DWM の枠管理外 |
| `GWLP_WNDPROC` を差し替えて `WM_NCPAINT` で塗り直す | 差し替えは成功する (戻り値が非ゼロ) のに、以後の `WM_NCPAINT` が届かない。描画ログが 1 回しか出ない |
| `SetWindowRgn` で領域を 1 px 内側へ縮める | 見た目が変わらない |

途中で分かった周辺事実:

- `WindowFromDC` は 0 を返す。`WM_DRAWITEM` の DC はウィンドウに
  紐づかないメモリ DC なので、そこからハンドルは辿れない
- `WM_DRAWITEM` の時点ではポップアップがまだ非表示。項目を測って
  描いてから表示される順序なので `IsWindowVisible` では拾えない
- `FrameRect` は right / bottom を含まない。幅・高さをそのまま渡すと
  右辺と下辺が 1 px 残る

**判定には等倍のスクリーンショットを使うこと。** 全画面を縮小した画像では
1 px の枠が潰れて周囲に溶け、直っていないのに直ったように見える。
実際にこれで何度も誤認した。

### アイコンは要求寸法以上のイメージリストから引く
`SHGFI_SMALLICON` / `SHIL_SMALL` は 16px を返す。これを 32px へ引き伸ばすと
輪郭がにじむ。**QAP のアイコンがきれいなのは最初から必要な解像度で取っているから。**
`SHIL_LARGE` (32) / `SHIL_EXTRALARGE` (48) / `SHIL_JUMBO` (256) を寸法で選び分け、
拡大ではなく縮小になるようにする (`image_list_for`) 。添字はリスト間で共通なので、
`SHGetFileInfo` で得た `iIcon` をそのまま別のリストへ渡せる。

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
Windows の既定フォルダーハンドラーで開く。Quick Launch のホットキーからはフォルダ・
Open Windows・ブックマーク (`b `) ・アプリ (`a `) を横断検索できる。実機で確認済み。

実装済み:
- FR-1 トリガー (中ボタン・ホットキー・トレイ左クリック・除外プロセス・ドラッグ判定)
- FR-2 メニュー表示 (階層・アイコン・数字アクセラレータ・グレー表示)
- FR-3 4 種類の項目、FR-4 newWindow / reuse、FR-5 変数展開
- FR-6 管理画面 (項目・ユーザー変数・トリガー設定の編集、保存後の即時反映)
- FR-7 設定の読み書き (原子的保存・パース失敗時の継続起動)
- FR-8 常駐 / トレイメニュー / 二重起動抑止 / 自動起動
- FR-9 Quick Launch (フォルダ・Recent/Frequent・Current Windows・ブックマーク検索)

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
