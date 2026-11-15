# waypoint 仕様書: Azure DevOps 検索 (`az `)

[spec.md](spec.md) の FR-9.18 系を分けたもの。スコープ判定は spec.md 02 章の「Azure DevOps 検索」「Azure DevOps 作成ページ」の行にある。FR 番号は spec.md と共通で、ここでも変えていない。

要点:

- `az ` で設定済みプロジェクトの PR・Pipeline・Work Item を検索する。通常入力の横断検索には含めない (FR-9.2)
- 打鍵ではメモリ上の索引だけを引く。API は `Ctrl+Enter` などの明示操作と、バックグラウンド同期でだけ呼ぶ
- PAT は Windows Credential Manager にだけ置く。無ければ Azure CLI のトークンを使う
- 書き込み API は呼ばない。作成はブラウザの作成ページを開くだけ

---

## 1. コマンドと検索

### FR-9.18 `az ` モードの基本

- 入力が `az ` (半角スペース込み) で始まる間は通常の検索対象を隠し、設定済み Azure DevOps プロジェクトの候補だけを検索する
- `az ` の直後は「My review requests」「My active PRs」と、設定済みの監視リポジトリごとの同等ショートカットを先頭に表示する。選ぶと Active PR の `needs-review` / `author` 条件を検索欄へ補完する。Repository 別ショートカットは `repo:<organization>/<project>/<repository>`（各セグメントを URL エンコード）を補完し、3 要素の完全一致でローカル・Live の両方を限定する
- 属性トークン（順不同・任意個数で組み合わせられる。既知の属性トークンが尽きた時点から先を検索語として扱う）:

  | サブコマンド | 別名 | 属性トークン |
  |---|---|---|
  | `az pr` | `prs` | `active` / `completed`（`complete`）/ `abandoned`（`abandon`）/ `all`（状態、省略時 all）、`mine`（`me`、自分が作成またはレビュー担当）、`author`（`authored`、自分が作成者）、`reviewer`（`review`、自分が reviewer）、`live`（Live 検索用の互換トークン） |
  | `az wit` | `wi` / `workitems` | `live`。入力だけでは API を呼ばず `Ctrl+Enter` で実行する |
  | `az pipeline` | `pipelines` / `pipe` / `build` / `builds` | `failed`（`fail`）/ `definition`（`definitions` / `def`）/ `all`（省略時 all） |
  | `az project` | `projects` | なし。プロジェクトを検索する |

- Live 検索:
  - `az pr` / `az wit` / `az pipeline` の入力中に `Ctrl+Enter` を押すと、検索語が空でも現在の属性条件で Live 検索を実行する
  - サブコマンドを伴わない `az <検索語>`（横断検索）でも `Ctrl+Enter` は Live 検索を実行し、PR / Work Item / Pipeline の 3 種を同時に問い合わせて結果を PR → Work Item → Pipeline の順にマージして表示する。到着した種別から順に表示し、全種が 0 件のときだけ「見つからない」旨を出す
  - PR の状態を省略した Live 検索は API 負荷を抑えるため Active のみを取得し、3 状態すべてを取得する場合は `all` を明示する
  - Live 検索は種別ごとに同時 1 件だけ実行し、同一条件の完了後 2 秒以内の再実行も無視する
  - `az project` / `az optimize` は検索対象を持たないため Live 検索の対象外で、`Ctrl+Enter` は通常の Enter と同じく候補を実行する
- 操作ヒントは `az` モード中の検索バーに表示する。通常の Enter はキャッシュ候補の選択・実行に使う
- 候補を選ぶと対応する Azure DevOps の Web ページを開く
- タイトル検索では入力した語句がその順序で連続する候補を最優先し、次に全語がタイトルへ一致する候補を並べる。4 文字以上の英数字語は挿入・削除・置換・隣接文字の入れ替えが 1 文字だけのタイプミスも一致として扱う

### FR-9.18.4 自分が関与する項目の優先

`az` 検索は自分が関与する項目を優先する。PR は作成者またはレビュー担当が自分の場合（`is_mine`）、キャッシュの `az` 横断検索および `az pr` 系コマンドで、タイトル一致度が同じなら自分が関わる Active PR を優先する（Live の順位は FR-9.18.10）（`mine` 属性トークンによる絞り込みとは独立に適用する）。プロジェクトごとに関心のある Area Path を設定でき（最近更新された Work Item の WIQL 取得条件に Iteration Path とともに組み込む。文字列 Live 検索は Project 全体、ID 完全一致は Project 内の ID を対象とし、Area / Iteration では絞らない）、設定済みなら一致する Work Item だけを対象にする。未設定のプロジェクトはプロジェクト全体を対象にする（従来どおり）

### FR-9.18.5 PR のショートカットと属性

`az ` の先頭には「My review requests」（自分が reviewer かつ未投票）、「My active PRs」（自分が作成者）、「Waiting for others」（自分が作成者で required reviewer の未投票または差し戻しが残る）、「Stale PRs」（作成から14日以上）をこの順に置く。設定済みの監視リポジトリがあれば各リポジトリの review requests / my PRs / waiting for others も続けて出し、その後に既存 Azure コマンド候補を置く。`az pr` の属性トークンは `needs-review` / `needsreview`、`waiting` / `waiting-for-others`、`draft` / `drafts`、`ready` / `ready-to-complete`、`stale` を使える。入力時はキャッシュで絞り込み、明示的な Live 検索にも同じ属性・Repository スコープを適用する。入力経路で API は呼ばない。PR 候補には organization / project / repository、状態、source → target branch、作成者を表示し、`Ctrl+C` は従来どおり URL、`Ctrl+Shift+C` は source branch をコピーする。Enter は PR URL を開く。

### FR-9.18.7 Recent 区分

`az ` の直後（検索語なし）は、Quick Launch の使用頻度履歴（`quick_launch_history`）に選択記録のある PR / Work Item を、種別ごとに上位 3 件ずつ「Recent」区分として先頭に出す。見出しは空クエリ時の区分一覧（FR-9.12）と同じ形式で、続けて「Commands」区分（FR-9.18 / FR-9.18.5 のショートカットとコマンド候補）を置く。順位は履歴の並び（使用回数の多い順、同数なら最終選択の新しい順）に従い、種別内で数える。履歴に該当が無い（初回）、または対象がキャッシュに無い場合は Recent 区分を出さず、従来どおり Commands だけを見出しなしで出す。履歴の保存形式は変えず、入力経路で API・SQLite・ファイル I/O は行わない（索引と使用履歴のメモリ上の照合だけ）。Pipeline は永続キャッシュを持たず索引に載らないため対象外。`settings.quickLaunch.includeAzureRecent`（既定オン）で無効化できる。設定画面の Quick Launch タブに「Azure recent」チェックを置く

### FR-9.18.8 `az new` (作成ページを開く)

`az new` はまず作成する種別（Bug / Task / User Story / Pull request と同期済みの独自 Work Item Type）を重複なしで表示する。種別を Enter で選ぶと検索欄を補完し、その種別を作成できる Project の一覧へ進む。Project は priority 順（小さい値を優先、同値は設定順）に並べ、名前を主表示、組織名と操作内容を補足表示にする。`az new bug` / `task` / `story` / `pr` を直接入力しても Project の選択から始められる。Work Item の Project を選ぶと `.../_workitems/create/{type}` をブラウザで開く。PR の Project を選ぶと、その Project の監視リポジトリ一覧へ進み、リポジトリを選んで `.../_git/{repo}/pullrequestcreate` を開く。監視リポジトリが未設定なら Project の `.../_git` を開いてブラウザで選択する。Type 名は Project ごとの同期済み一覧を使い、未同期の場合は標準の Bug / Task / User Story を暫定表示して型の対応が Project に依存する旨を示す。Azure 無効・Project 未設定・対象スコープ無しなら設定を開く候補を表示する。不一致の入力には操作例を表示する。組織・Project・リポジトリ・型の URL セグメントはエンコードする。候補は索引構築・Azure 更新時にメモリ上へ組み立て、入力・表示時に API・DB を呼ばない。各選択は Enter / Ctrl+Enter で実行し、書き込み API は呼ばない。`az ` のコマンド補完に `az new` を加える。

### FR-9.18.10 検索契約

未知の1語がサブコマンド名の補完に一致しても、補完と通常の Azure 検索結果を併記する。属性は検索語より前に置き、最初の未知語以降はすべて検索語とする。関係属性は AND、状態属性を複数指定した場合は最後を採用する。`repo:` スコープは PR の属性の直後に置く。キャッシュ PR の状態省略は all、Live の省略は Active のままとし、入力中に差を表示する。ローカルはタイトル・所属・URL・branch を照合するが、PR / Pipeline Live はタイトル、WI の文字列 Live は全文検索 API を使う。横断キャッシュは PR / WI / Project、横断 Live は PR / WI / Pipeline。Live は同じ検索の更新ではなく追加取得である。検索結果の先頭に、検索範囲・取得上限・部分失敗を非選択の説明行として表示する。上限は Project ごとに WI 文字列300件 / 最近更新800件、Pipeline 定義1,000件 / 直近 Build100件、PR履歴 Live365日 / 状態ごと2,000件（番号指定は単体取得）。`failed` は取得した直近100 Buildを絞り込むもので、失敗Buildを100件取得するものではない。取得失敗と0件を区別し、一部成功時も失敗Projectを表示する。順位はローカルがタイトル品質→状態・自分との関係→一致スコア・使用履歴、PR/Pipeline Liveがタイトル品質→Project priority→名前、WI文字列LiveがProject priority→名前、横断LiveはPR→WI→Pipelineの連結とする。UIスレッドでAPI・DB・ファイルを読まない。

---

## 2. キャッシュと同期

### FR-9.18.1 共有キャッシュと PR 番号指定

`az wit `（`wi` / `workitems`）と `az pr`（`prs`）の Active な候補は、waypoint と DevDeck（別リポジトリの Azure DevOps ダッシュボードアプリ）のどちらの内部スキーマにも属さない中立な共有キャッシュ（`%APPDATA%\\AzDoSharedCache\\cache.db`）を参照する。両アプリが同じ組織を独立にポーリングして API 呼び出しが重複するのを避けるため、この共有キャッシュが直近更新済み（自分でも DevDeck でも）なら waypoint は自分の API 呼び出しをスキップし、古ければ取得したうえで共有キャッシュにも書き込む。is_mine の判定は共有キャッシュの `created_by_id` / レビュアー一覧を自分の認証ユーザー ID（`identity` テーブルに事前解決済み、キー入力経路ではネットワークに触れない）と突き合わせて waypoint 側で計算する。`az wit` のローカルキャッシュが 0 件なら Live 検索候補を表示するが、API は候補の Enter または `Ctrl+Enter` の明示操作でのみ非同期に問い合わせ、ライブ検索結果は永続化しない。

PR 番号（数字だけの検索語）を指定した `az pr <番号>` / `az <番号>` で、その番号の PR がキャッシュ候補に無いときは末尾に「Look up PR <番号>」候補を出し、Enter（または `Ctrl+Enter`）で一覧 API のスキャンではなく PR 単体取得 API（`GET {org}/{project}/_apis/git/pullrequests/{id}`）を監視プロジェクトごとに 1 回ずつ直接叩く。一覧の打ち切り（1 年 / 2,000 件）より古い PR も取得でき、見つかった PR はリポジトリ名を含む Web URL（`.../_git/<repo>/pullrequest/<id>`）で開く。番号は状態より強い指定として扱い、状態属性トークンでは絞り込まない。番号の前に `#` を付けた `az #<番号>` / `az pr #<番号>` は完全一致指定とし、先頭 ID が一致する候補だけを返す（数字だけの検索語で拾う部分一致・breadcrumb / URL 一致は返さない）。`az wit #<番号>` も同様で、キャッシュは ID 完全一致で絞り、Live 検索は全文検索ではなく WIQL の `[System.Id] = <番号>`（監視プロジェクト内）で直接引く。404 は「そのプロジェクトに無い」とみなし失敗扱いにしない。

PR の Completed/Abandoned 履歴（過去 90 日ぶん、DevDeck の対象外）は引き続き waypoint 自身の `%APPDATA%\\waypoint\\azure_devops.db` にキャッシュし、起動時・設定再読み込み時・12 時間おきにバックグラウンド同期する。Pipeline は永続キャッシュを持たず、Live 検索候補の Enter または `Ctrl+Enter` の明示操作で問い合わせる

### FR-9.18.3 PR 履歴キャッシュの鮮度と再試行

waypoint 自身が同期する PR 履歴のキャッシュにはプロジェクト別の最終成功時刻と失敗状態を保持する。設定画面とトレイメニューで鮮度を表示し、手動更新できる（Active PR / Work Item の鮮度は共有キャッシュの `sync_state`（自分か DevDeck、どちらが最後に更新したか）に依存するため、この表示の対象外）。同一の同期要求は重複実行しない。HTTP 429 および 5xx は短いバックオフを伴い再試行し、`Retry-After` 応答ヘッダーがあれば固定バックオフより優先して指定秒数を待つ。失敗時も直前の成功キャッシュを残す

### FR-9.18.9 共有キャッシュの障害時

共有キャッシュ（`AzDoSharedCache\cache.db`）は waypoint の可用性を DevDeck 側のスキーマ変更や書き込みタイミングに依存させない設計にする。DB ファイルが開けない、または想定と異なるスキーマでクエリが失敗した場合は、その呼び出しだけを空扱いにして通常表示を継続する（クラッシュしない）。waypoint 自身が保持する接続はクエリ失敗を検知した時点で破棄し、次回呼び出しで新しい接続を開き直す。共有キャッシュ自体が開けない場合でも、waypoint 自身の PR 履歴キャッシュ（FR-9.18.3）は独立した DB のため引き続き機能する（Active PR / Work Item のみ 0 件表示になる）。DevDeck 側の変更を待たずに waypoint 単独で切り離せる設計を維持する

---

## 3. 認証と設定

### FR-9.18.2 PAT・Entra トークンと監視対象の設定

- Azure DevOps の PAT は設定ファイルへ保存せず、組織ごとに Windows Credential Manager へ保存する
- PAT が保存されていない組織では、Azure CLI (`az account get-access-token --resource 499b84ac-1321-427f-aa17-267ca6975798`) で取得した Microsoft Entra のトークンを Bearer 認証に使う。`az` の起動は数秒かかるため、同期・Live 検索・設定画面の API 呼び出しなどバックグラウンドの経路でだけ呼び、トークンは期限の 5 分前まで使い回す。PAT が保存されていればそちらを優先する。Entra に接続されていない (Microsoft 個人アカウントで作った) 組織はこのトークンを受け付けないため、その場合は引き続き PAT が必要
- 設定画面の専用プロジェクト選択画面では PAT 保存後に組織のプロジェクト一覧を取得し、文字列で絞り込みながら複数の監視対象をチェックで選べる。一覧でチェックした行を選ぶと詳細パネルが開き、別名・優先度・同期スコープ（PR / Pipeline / Work Item）・興味のある Area Path を GUI 上で編集できる
- Area Path は Azure DevOps の Area 階層 API から取得したツリーをチェックボックスで選ぶ（検索フィルタ付き）。テキスト DSL を手打ちする経路は持たない
- 「Suggest from my assigned work items」ボタンを押すと、選択中プロジェクトで自分に割り当てられた Work Item を WIQL (`[System.AssignedTo] = @Me`) で取得し、その Area Path を件数の多い順に候補として表示する（チェックボックスで選択に追加できる）。ボタンを押すまで API は呼ばない

### FR-9.18.2.1 `az optimize`

`az optimize`（`suggest` / `rank` でも入れる）は Quick Launch から直近アクティビティを基にした最適化を起動する専用コマンド。検索対象を持たず確定候補を 1 件だけ表示し、選ぶと常駐部がバックグラウンドで監視中の全プロジェクトを集計する。設定画面は開かず、ユーザーの選択も求めない。Azure DevOps の Account My Work Recent Activity REST API から、自分の直近90日間の Work Item を取得し、プロジェクトごとに Iteration Path を件数順で集計する。各プロジェクトには最頻の Iteration Path を `interestIterations` として自動保存し、Work Item のキャッシュ取得と空クエリの Live 取得ではその Iteration 配下に絞る（文字列 Live は Project 全体）。プロジェクトの `priority` はアクティビティ件数の多い順（同数時は既存 priority 順）に振り直す。取得失敗した Project の priority と interestIterations は保持し、その priority 値を予約して成功した Project のみ並べ替える。一部成功でも失敗した Project は活動0件と区別する。全件失敗または全体で90日内の活動が0件なら保存しない。処理結果はログとトレイ通知で表示する。REST API で展開されない WIQL の UI マクロには依存しない。実行するまで API は呼ばない

---

## 4. 表示

### 補完候補・ショートカットのアイコンと補足行

`az ` の補完候補・PR ショートカットにも検索対象に対応する種別アイコンを表示する。PR は分岐、Pipeline は実行、Project はグリッド、Bug は虫、Task はチェックとして描く。候補の補足行では状態表記を英語の短いラベルに揃え、作成者・担当者をブランチより先に表示する。検索・実行に使う元データは変更しない。

### FR-9.18.6 種別・状態アイコンと照合範囲

- **アイコンの形**: `az ` モードの候補は、行頭アイコンで種別と状態を示す。アイコンは種別色で塗った角丸タイルに白い記号を載せたもので、縁はアンチエイリアスする。種別と状態は URL から推定せず、候補を作る時点の情報（`Entry::azure`）を使う。持たない候補だけ、従来どおり URL から種別を推定する
- **種別の色と記号**: PR が紫の ⇄、Pipeline が緑の ▶、Project がシアンの ▦。Work Item は Type ごとに Azure DevOps 本家の配色に合わせ、Bug が赤の ✱、Task が黄の ✓、Feature が紫の ★、Epic が橙の ♛、User Story とその他の Type が青の ◆
- **Type の色の同期**: 同期時（バックグラウンド）にプロジェクトごと 1 回だけ `GET {org}/{project}/_apis/wit/workitemtypes` から取得した Type の色（`color`）とアイコン（`icon.id`）がある場合はそれを優先する。これにより Agile / Scrum / CMMI / Basic や独自の Type も Azure DevOps の画面と同じ色になる。記号は `icon.id` に対応する組込みの記号を使い、対応が無ければ Type 名から決めた上記の記号、それも無ければ Type 名の頭文字にする。取得結果は `azure_devops.db` に保存して次回起動から使い、表示・入力の経路では API を呼ばない。取得できない場合は上記の配色へフォールバックする
- **状態マーカー**: タイル右下には、状態を示すマーカーを行の地の色で縁取って 1 つだけ重ねる。マーカーは色だけに頼らず、塗り方と記号の形でも区別できるようにする（グレースケールでも判別できる）。優先順は、自分のレビュー待ち（橙の点）、種別ごとの状態の順

  | 種別 | 状態 → マーカー |
  |---|---|
  | PR | Completed が緑の塗り円 + チェック、Abandoned が灰の塗り円 + ×、Draft が灰の輪、自分が作成者で required reviewer が全員承認済み（`ready_to_complete`）が緑の輪 + チェック（Draft が先）、それ以外の Active は出さない |
  | Pipeline | succeeded が緑の塗り円 + チェック、failed が赤の塗り円 + ×、inProgress（cancelling を含む）が青の塗り円 + 回転矢印、canceled が灰の塗り円 + 斜線、partiallySucceeded が琥珀の塗り円 + !、定義や未開始などそれ以外は出さない |
  | Work Item | State を Process Template 間で寄せて、New / Proposed / To Do / Approved / Design が灰の輪、Active / Committed / Doing / In Progress が青の点、Resolved / In Review / Testing が橙の半円、Closed / Done が緑の塗り円 + チェック、Removed が灰の塗り円 + ×、未知の State は出さない |

- **完了済みの表示**: 完了済みの候補（PR の Completed / Abandoned、Work Item の Closed / Done / Removed）は、タイルと名前を薄く描いたうえでマーカーは通常の濃さで出す（完了と破棄を区別するため）
- **照合範囲**: 検索では、breadcrumb と URL は部分文字列一致だけで照合し、fuzzy（語の文字が順に含まれるかどうかの一致）は名前にだけ当てる。breadcrumb 共通の前置き `Azure DevOps — ` と URL 共通の前置き `https://dev.azure.com/` は照合から外す（`az azure` で全件が一致しないように）。PR の source branch は breadcrumb の検索キーに足し、表示に含まれない場合（PR 履歴・Live 検索の結果）でも検索できるようにする。`az <語>` のサブコマンド補完はコマンド名だけで照合し、説明文には一致させない
- **並び順**: タイトル一致度が同じ候補は、自分が関わる Active PR、その他の未完了、完了済みの順に並べる（状態はタイトル一致度より後に効く）
- **2 行目の組み替え**: 候補の 2 行目は、どのプロジェクト / リポジトリの項目かが一目で分かるよう、表示するときだけ breadcrumb を組み替える。定型の前置き `Azure DevOps — ` と組織名を外し、`project/repo` を先頭にアクセント色で出して、状態・ブランチ・作成者などを中黒で続ける。PR 履歴の breadcrumb はリポジトリ名を含まないため、URL の `_git/<repo>/` から補う。検索キーは元の breadcrumb から作るので、この組み替えの影響を受けない
