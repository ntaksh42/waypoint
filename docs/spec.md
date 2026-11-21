# waypoint 仕様書

- 版: `Cargo.toml` の `version`
- 対象: Windows 10 22H2 以降

Windows の常駐型ランチャー。ホットキーで Quick Launch を開き、フォルダ・ウィンドウ・アプリ・ブックマークなどを検索して開く。

Azure DevOps 検索 (`az `) は [spec-azure.md](spec-azure.md) にある。

---

## 用語

| 用語 | 意味 |
|---|---|
| 項目 | 設定ファイルに登録したフォルダ・ファイルなど |
| 元ウィンドウ | Quick Launch を開く直前に最前面だったウィンドウ |
| 常駐部 | トレイに常駐する本体プロセス (`waypoint.exe`) |
| 管理画面 | 設定を編集する別プロセス (`waypoint-settings.exe`) |

---

## 3. 項目の型

| 型 | 選んだときの動作 |
|---|---|
| `folder` | フォルダを開く |
| `file` | 関連付けアプリで開く |
| `specialFolder` | 既知フォルダ (Downloads など) を開く |
| `submenu` | 子項目をまとめる。子の候補名にグループ名が付く |
| `separator` | 検索対象にしない |

## 4. フォルダの開き方

- `newWindow` (既定): 新しいウィンドウで開く
- `reuse`: 元ウィンドウ (エクスプローラー) の表示先を変える。元ウィンドウがエクスプローラーでなければ `newWindow`
- `Shift+Enter` は常に `newWindow`、`Ctrl+Enter` は常に `reuse`
- 開いたウィンドウはトリガーのあったモニターに置く

## 5. 変数

- `%NAME%`: 環境変数
- `{NAME}`: 設定ファイルのユーザー変数
- 解決できない変数を含む項目は候補に出さず、ログに記録する

## 6. 管理画面

### 項目の編集

- 左に候補グループのツリー、右に項目の一覧
- 項目の追加・編集・削除・複製・コピー＆ペースト
- 並べ替え: 上下ボタン、ドラッグ
- パス指定: ダイアログ、エクスプローラーからのドロップ
- 複数選択: `Ctrl+クリック` / `Shift+クリック` / `Shift+矢印` / `Ctrl+A`。削除・移動・複製はまとめて適用する
- 複数選択で Edit: 開き方とブランチ表示をまとめて変える
- Move...: 選んだ項目を別グループの末尾へ移す
- 検索 (`Ctrl+F`): 全グループから名前・パスで探す。結果を選ぶとその項目へ移る
- ツリーと Move... の移動先は名前で絞り込める
- 保存すると常駐部へ反映する。未保存で閉じると確認する

### その他の設定

- ユーザー変数の編集
- ホットキー: 文字で入力するか、実際に押して記録する (`Win+W` など予約済みのキーも可)
- フォルダの一括取り込み: 起点と深さ (0〜10) を指定し、サブフォルダをグループとして登録する。取り込み前に名前の変更と除外ができる。取り込み後は元フォルダと同期しない

### Edge お気に入りの編集 (6.16〜6.20)

- `waypoint-settings.exe --bookmarks` で開く。同時に 1 つまで
- 左にフォルダツリー (件数付き)、右に一覧。名前と URL はセルで直接編集できる
- 一括操作: 移動・置換 (正規表現可、プレビュー付き)・削除・並べ替え・名前順・重複 URL の削除
- フォルダの作成・名前変更・削除はツリーで行う。最上位フォルダは変えられない
- 絞り込み: 空白区切りの語 (AND) を名前・URL・フォルダに部分一致。重複だけの表示、配下を含めるかを切り替えられる
- `Ctrl+Z` で 30 件まで戻せる。Redo は無い
- 起動時と一括操作の前に `%LOCALAPPDATA%\waypoint\bookmark-backups\` へバックアップする (新しい 30 件を残す)
- 読み書きはブラウザ拡張 (Waypoint Tab Bridge) 経由。Chrome は対象外

## 7. 設定ファイル

- 場所: `%APPDATA%\waypoint\config.json`。無ければ既定値で作る
- 保存は一時ファイルへ書いてから置き換える。項目の順序を保つ
- 読めない JSON のときは候補なしで起動し、トレイ通知で知らせる。ファイルは上書きしない
- 利用履歴: `%APPDATA%\waypoint\history.json`
- `shared.json`: 保存時に `settings` を `%APPDATA%\waypoint\shared.json` へも書く。起動時にあれば `config.json` の `settings` を上書きする。`items` と `variables` は含めない
- 存在しないパスの自動削除: 起動時と 10 分ごとに `folder` / `file` を確かめ、ドライブや共有のルートはあるのに対象が無い項目を削除する。ルートが見えない項目は残す

## 8. 常駐

- 起動するとトレイに常駐する
- トレイの右クリック: 設定を開く / 設定を再読み込み / 終了
- 二重起動しない。2 つ目は既存プロセスに Quick Launch を開かせて終わる
- Windows 起動時の自動実行を切り替えられる

---

## 9. Quick Launch

### 表示

- 9.1: ホットキー (既定 `Alt+Space`) で開く
- 9.11: 黒基調。選択行は青、検索語に一致した部分は下線付き太字
- 9.12: 各行はアイコン・名前・パンくず・パス。右端に種別 (選択行では Enter の動作)。下端に使えるキーと選択位置 (`3 / 24`)
- 表示件数は 12〜24 (既定 12)
- 9.12.1: 表示するモニターは `primary` (既定) か `cursor`。作業領域の中央に出す

### 検索

- 9.2: 入力すると、登録項目・Recent / Frequent Folders・開いているウィンドウ・ブックマーク・アプリをまとめて検索する
- 9.3: 名前とパンくずに大文字小文字を無視して部分一致。空白区切りは AND
- 9.4: パスも検索対象にできる (設定)
- 9.5: 並び順は 完全一致 → 先頭一致 → 単語境界 → 部分一致 → パンくず → パス → あいまい一致。同点はメニュー順
- 9.6: `showBranch` のフォルダは Git ブランチ名を右端に出す
- 9.27: かなだけの名前は、ローマ字・ひらがな・カタカナのどれで打っても一致する (`disuko-do` → 「ディスコード」)

### キー

| キー | 動作 |
|---|---|
| `Up` `Down` `Home` `End` `PageUp` `PageDown` | 選択を移す |
| `Enter` | 実行 |
| `Esc` / フォーカスが外れる | 閉じる |
| `Shift+Enter` / `Ctrl+Enter` | フォルダを `newWindow` / `reuse` で開く |
| `Ctrl+Shift+Enter` (9.8.1) | 選んだ候補を設定に登録する。対象はフォルダ・ファイル・`f ` の結果・アプリ。登録済みなら何もしない |
| `Ctrl+C` (9.8.2) | パス (URL の候補なら URL) をコピー |
| `Ctrl+E` (9.8.3) | エクスプローラーで選択表示 |
| `Ctrl+Alt+Enter` (9.8.4) | 管理者として実行。対象はアプリと `f ` の結果 |
| `Ctrl+W` (9.8.5) | 選んだウィンドウを閉じる。Quick Launch は開いたまま |
| `Tab` (9.26) | 選んだフォルダの中へ潜る |

### プレフィックス

入力の先頭に置くと、検索対象を絞る。

| 入力 | 対象 | Enter | 無効化の設定 | 番号 |
|---|---|---|---|---|
| `b ` | Chrome / Edge のブックマーク | ブラウザで開く | `includeBookmarks` | 9.13 |
| `h ` | Chrome / Edge の閲覧履歴 | ブラウザで開く | `includeBrowserHistory` | 9.17 |
| `t ` | Chrome / Edge の開いているタブ | タブへ切り替え | — | 9.17.1 |
| `a ` | スタートメニューのアプリ | 起動 | `includeApps` | 9.14 |
| `w ` | 開いているウィンドウ | 切り替え | `includeOpenWindows` | 9.15 |
| `k ` | 実行中のプロセス | 確認なしで終了 | — | 9.15.2 |
| `ps ` | フォルダ | PowerShell 7 (Windows Terminal) で開く | — | 9.15.1 |
| `ed ` | フォルダ | エディターで開く | — | 9.15.3 |
| `cc ` | フォルダ | Claude Code を起動 | — | 9.15.4 |
| `cx ` | フォルダ | Codex CLI を起動 | — | 9.15.5 |
| `cs ` | Claude Code / Codex の過去セッション | 再開 | — | 9.15.6 |
| `vs ` | VS Code / Cursor の最近のワークスペース | そのエディターで開く | `includeEditorWorkspaces` | 9.23 |
| `f ` | Everything の検索結果 | 開く | `includeEverything` | 9.16 |
| `??` | Web 検索 | 検索結果を開く | `includeWebSearch` | 9.21 |
| `az ` | Azure DevOps | Web ページを開く | `azureDevOps.enabled` | 9.18 |

閲覧履歴・Everything・Azure DevOps は、プレフィックスを付けたときだけ検索する。

### 各プレフィックスの補足

- `b ` `h `: 同じ URL は 1 件にまとめる (http/https、末尾の `/`、トラッキング用クエリの差を無視)。`http` / `https` 以外の URL は開かない
- `t `: ブラウザ拡張 (Waypoint Tab Bridge) が要る。未導入なら導入案内を開く候補を 1 件出す。タブの情報は保存しない
- `w `: タイトルとプロセス名で探す (`w chrome`)
- `k `: waypoint 自身は出さない。終了に失敗してもエラーは出さない
- `ps `: `wt.exe -d <folder> pwsh`。無ければ Windows PowerShell
- `ed `: `editorCommand` (既定 `code`) にフォルダを渡す
- `cc `: フォルダを選ぶと `cc <folder> ` まで補完する。続けてセッション名 (省略可、空白不可) を打って Enter で起動する
- `cx `: フォルダを選ぶとすぐ起動する
- `cs `: 新しい順に並べる。Claude Code は `claude --resume <id>`、Codex は `codex resume <id>` で再開する。タイトルの無いセッションは出さない
- `vs `: リモートは `Remote` タグで区別する。Insiders・VSCodium・Windsurf は対象外
- `f `: `Alt+C` (大文字小文字) / `Alt+W` (単語単位) / `Alt+R` (正規表現) を切り替えられる。Everything が無ければ 0 件
- `??`: `?? rust` で検索する。空なら検索エンジンのトップを開く。エンジンは Google (既定) / Bing / DuckDuckGo / Brave

### パス入力

- 9.24: `C:\` や `\\server` で始まる入力は、そのパスを開く候補を先頭に出す
- 9.25: パス入力中の `Tab` はサブフォルダ名を補完する。もう一度 `Tab` / `Shift+Tab` で候補を巡る。それ以外の `Shift+Tab` は 1 階層上へ
- 9.26: フォルダ候補で `Tab` を押すと、その中のサブフォルダを一覧する

### 固定の候補

| 候補 | 動作 | 番号 |
|---|---|---|
| Settings | 管理画面を開く | 9.20 |
| Help | 機能一覧のページ (`help\index.html`) をブラウザで開く | 9.22 |
| Edit Bookmarks | Edge お気に入りの編集画面を開く | 9.28 |

---

## 設定ファイルの形式

```json
{
  "version": 1,
  "variables": {
    "Proj": "D:\\work\\projects"
  },
  "settings": {
    "quickLaunch": {
      "hotkey": "Alt+Space",
      "includeRecentFolders": true,
      "includeFrequentFolders": true,
      "includeOpenWindows": true,
      "includeBookmarks": true,
      "includeBrowserHistory": true,
      "includeApps": true,
      "includeEverything": true,
      "azureDevOps": { "enabled": true, "projects": [] },
      "searchPaths": false,
      "editorCommand": "code",
      "visibleResults": 12,
      "monitor": "primary"
    },
    "startWithWindows": false
  },
  "items": [
    { "type": "folder", "name": "waypoint", "path": "{Proj}\\waypoint", "open": "reuse", "showBranch": true },
    { "type": "separator", "name": "開発" },
    {
      "type": "submenu",
      "name": "ビルド出力",
      "items": [
        { "type": "folder", "name": "Debug", "path": "{Proj}\\waypoint\\target\\debug" }
      ]
    },
    { "type": "specialFolder", "name": "ダウンロード", "knownFolder": "Downloads" }
  ]
}
```

| フィールド | 対象 | 内容 |
|---|---|---|
| `type` | 全部 | `folder` / `file` / `specialFolder` / `submenu` / `separator` |
| `name` | 全部 | 表示名 (`separator` は省略可) |
| `path` | `folder` `file` | パス。変数を使える |
| `knownFolder` | `specialFolder` | `Downloads`、`Desktop` など |
| `items` | `submenu` | 子項目 |
| `open` | `folder` `specialFolder` | `newWindow` (既定) / `reuse` |
| `icon` | `folder` `file` | アイコンのパス |
| `showBranch` | `folder` `submenu` | Git ブランチ名を出す。`submenu` に付けると配下に効く |

---

## 非機能要件

| 項目 | 目標 |
|---|---|
| Quick Launch の表示 | ホットキーから 50ms 以内 |
| 常駐メモリ | 60MB 以下 |
| 起動 | ログオンから 1.5 秒以内にトレイ常駐 |
| DPI | Per-Monitor V2 |
| ダークモード | システムテーマに従う |
| パス長 | 260 文字超に対応 |
| ログ | `%APPDATA%\waypoint\log.txt` (起動・設定読み込み・エラー) |
