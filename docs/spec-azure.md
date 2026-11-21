# waypoint 仕様書: Azure DevOps 検索 (`az `)

[spec.md](spec.md) の 9.18 系。

- `az ` で、設定したプロジェクトの PR・Pipeline・Work Item を検索する
- 入力中はキャッシュだけを検索する。サーバーへの問い合わせ (Live 検索) は `Ctrl+Enter` で行う
- 書き込みはしない。作成はブラウザの作成ページを開くだけ

---

## 9.18 コマンド

| 入力 | 別名 | 対象 |
|---|---|---|
| `az <語>` | | PR・Work Item・Project をまとめて検索 |
| `az pr` | `prs` | PR |
| `az wit` | `wi` `workitems` | Work Item |
| `az pipeline` | `pipelines` `pipe` `build` `builds` | Pipeline |
| `az project` | `projects` | プロジェクト |
| `az new` | | 作成ページを開く (9.18.8) |
| `az optimize` | | 最近の活動から、全プロジェクトの優先度と Work Item の Iteration Path を設定し直す (9.18.2.1) |

- Enter で選んだ項目の Web ページを開く
- `Ctrl+Enter` で Live 検索する。`az <語>` では PR → Work Item → Pipeline の順に並べる
- 4 文字以上の英数字の語は 1 文字のタイプミスも一致とみなす
- `az "<語>"` のように `"` で始めると、コマンド名と同じ語 (`build` や `new` など) も `az <語>` と同じ横断検索の検索語になる。前後の `"` は検索語に含めない

### 属性

コマンドの直後に置き、検索対象を絞る。順不同で組み合わせられる。属性でない最初の語から後ろが検索語になる。

| コマンド | 属性 |
|---|---|
| `az pr` | 状態: `active` / `completed` / `abandoned` / `all` (省略時 all、Live では active。省略して Live 検索したときは、結果の先頭に active だけを検索したことと `all` で広げられることを表示する) |
| | 関係: `mine` (作成者かレビュアー)・`author`・`reviewer` |
| | その他: `needs-review`・`waiting`・`draft`・`ready`・`stale`・`repo:<org>/<project>/<repo>` |
| `az pipeline` | `failed`・`definition`・`all` |

`repo:` も属性の 1 つで、ほかの属性の前後どちらに置いてもよい (`az pr repo:o/p/r mine foo` と `az pr mine repo:o/p/r foo` は同じ)。

### 番号指定

- `az pr 123` / `az 123`: キャッシュに無ければ「Look up PR 123」候補を出し、Enter でサーバーから取り寄せる
- `az #123` / `az pr #123` / `az wit #123`: ID が完全に一致するものだけを出す

## 9.18.5 `az ` 直後の候補

検索語なしの `az ` では、次の順に出す。

1. Recent (9.18.7): 最近選んだ PR と Work Item を各 3 件。`includeAzureRecent` で消せる
2. My review requests: 自分がレビュアーで未投票の PR
3. My active PRs: 自分が作った PR
4. Waiting for others: 自分が作った PR で、必須レビュアーの承認待ちが残るもの
5. Stale PRs: 作成から 14 日以上の PR
6. 監視リポジトリごとの同じショートカット
7. コマンドの候補

PR の候補では `Ctrl+C` で URL、`Ctrl+Shift+C` でソースブランチ名をコピーする。

## 9.18.4 自分に関わる項目を上に出す

- タイトルの一致度が同じなら、自分が関わる Active PR → その他の未完了 → 完了済みの順に並べる
- プロジェクトごとに Area Path を設定すると、その Work Item だけを対象にする

## 9.18.8 `az new`

1. 作る種別を選ぶ (Bug / Task / User Story / Pull request / プロジェクト独自の種別)
2. プロジェクトを選ぶ
3. Work Item は作成ページを開く。PR はさらにリポジトリを選んで作成ページを開く

`az new bug` / `task` / `story` / `pr` と打てば 2 から始まる。

## 9.18.10 検索結果の表示

- 結果の先頭に、検索した範囲・取得の上限・失敗したプロジェクトを表示する
- 取得の上限 (プロジェクトごと)

  | 対象 | 上限 |
  |---|---|
  | Work Item (文字列検索) | 300 件 |
  | Work Item (最近更新) | 800 件 |
  | Pipeline 定義 | 1,000 件 |
  | Build | 直近 100 件 |
  | PR (Live) | 365 日以内、状態ごと 2,000 件 |

---

## キャッシュ

- 9.18.1: Active な PR と Work Item は DevDeck と共有するキャッシュ (`%APPDATA%\AzDoSharedCache\cache.db`) を使う。開けなければ 0 件として表示を続ける (9.18.9)
- 完了・破棄した PR (過去 90 日) は `%APPDATA%\waypoint\azure_devops.db` に持ち、起動時と 12 時間ごとに更新する
- 9.18.3: 最後に同期した時刻を設定画面とトレイに出す。手動で更新できる。失敗しても前回のキャッシュを残す
- Pipeline はキャッシュしない。Live 検索だけ

---

## 認証と設定

### 9.18.2 認証

- PAT は組織ごとに Windows 資格情報マネージャーへ保存する。設定ファイルには書かない
- PAT が無い組織では Azure CLI (`az account get-access-token`) のトークンを使う
- 個人の Microsoft アカウントで作った組織では PAT が要る

### 監視するプロジェクト

- 設定画面でプロジェクトを選ぶ
- プロジェクトごとに別名・優先度・同期する対象 (PR / Pipeline / Work Item)・Area Path を設定する
- Area Path はツリーから選ぶ。「Suggest from my assigned work items」で自分の担当分から候補を出せる

### 9.18.2.1 `az optimize`

候補行の説明に、全プロジェクトの優先度と Iteration Path を上書きすることを書く。選ぶと、自分の直近 90 日の Work Item を集計し、次を自動で設定する。

- 各プロジェクトで最も多い Iteration Path を、Work Item の取得範囲にする
- 活動の多い順にプロジェクトの優先度を振り直す

取得に失敗したプロジェクトの設定は変えない。結果はトレイ通知で知らせる。

---

## 9.18.6 アイコン

行頭に種別の色のタイルを置き、右下に状態のマークを重ねる。

| 種別 | タイル |
|---|---|
| PR | 紫 ⇄ |
| Pipeline | 緑 ▶ |
| Project | シアン ▦ |
| Bug | 赤 ✱ |
| Task | 黄 ✓ |
| Feature | 紫 ★ |
| Epic | 橙 ♛ |
| User Story・その他 | 青 ◆ |

Work Item の色は Azure DevOps の種別設定があればそれに合わせる。

| 種別 | 状態のマーク |
|---|---|
| PR | 自分のレビュー待ち: 橙の点 / Completed: 緑のチェック / Abandoned: 灰の × / Draft: 灰の輪 / 完了できる: 緑の輪 |
| Pipeline | 成功: 緑のチェック / 失敗: 赤の × / 実行中: 青の矢印 / 取り消し: 灰の斜線 / 一部成功: 琥珀の ! |
| Work Item | 未着手: 灰の輪 / 作業中: 青の点 / レビュー中: 橙の半円 / 完了: 緑のチェック / 削除: 灰の × |

完了済みの項目は薄く表示する。
