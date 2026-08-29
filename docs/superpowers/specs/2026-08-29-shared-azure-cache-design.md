# Azure DevOps 共有キャッシュ設計 (waypoint ⇔ DevDeck)

日付: 2026-08-29
関連: `waypoint-devdeck-dedup-sync` セッションとの連携作業

## 背景

同一ユーザーが waypoint (本リポジトリ、トレイ常駐ランチャー) と DevDeck
(`C:\Users\aksh0\source\repos\DevDeck`、Tauri + React の Azure DevOps
ダッシュボード) を並行運用している。両アプリは同じ Azure DevOps
プロジェクトへ独立に PR / Pipeline / Work Item を同期しており、同じデータを
二重に API 取得している。

- waypoint: `%APPDATA%\waypoint\azure_devops.db` に軽量スキーマ
  (`candidates` + `project_state`) で保持。`src/azure_devops/cache.rs`
- DevDeck: 自前の豊富なスキーマ (`SCHEMA_VERSION=19`、PR/WorkItem/Commit 等)
  で保持し、Tokio ループで定期同期。GitHub 対応もあり組織モデルは waypoint
  より広い

## 検討した approach

| Approach | 内容 | 判定 |
|---|---|---|
| A. 新規の独立した共有キャッシュファイル | 両アプリの既存スキーマは変更せず、第三の SQLite ファイルへ軽量な投影だけを共有する | **採用** |
| B. 片方のアプリが常にオーナー | 実装は単純だが、オーナー側アプリが起動していないとキャッシュが更新されず依存関係が生まれる | 却下 |
| C. IPC でのリアルタイム共有 | 両方起動中でないと意味がなく、永続性もない | 却下 |

## 採用design (Approach A)

### 保存場所とスキーマ

`%LOCALAPPDATA%\AzDoSharedCache\cache.db`（waypoint 自身の `azure_devops.db`
とは別ファイル。両アプリの内部スキーマには一切手を入れない）。

```sql
CREATE TABLE shared_candidates (
    organization TEXT NOT NULL,
    project TEXT NOT NULL,
    kind TEXT NOT NULL,           -- 'pr' | 'pipeline' | 'wit'
    item_id TEXT NOT NULL,
    status TEXT NOT NULL,
    name TEXT NOT NULL,
    detail TEXT NOT NULL,
    url TEXT NOT NULL,
    is_mine INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (organization, project, kind, item_id)
);
CREATE TABLE shared_project_state (
    organization TEXT NOT NULL,
    project TEXT NOT NULL,
    refreshed_at INTEGER,
    written_by TEXT,              -- 'waypoint' | 'devdeck'
    included_pull_requests INTEGER NOT NULL DEFAULT 0,
    included_pipelines INTEGER NOT NULL DEFAULT 0,
    included_work_items INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (organization, project)
);
```

`included_*` は書き込んだアプリがそのプロジェクトで実際に同期した範囲
(waypoint なら `AzureDevOpsProject.include_pull_requests` 等) を記録する。
読む側はこれを見て「相手の同期が自分に必要な範囲を含んでいるか」を判定する
— 含んでいなければ相手の結果を採用せず自分で同期する。これが無いと、
自分は Pipeline も見たいのに相手が PR しか同期していなかった場合に
Pipeline のキャッシュを消してしまう (`replace_project_cache` は全置換のため)。

### dedup の判定

プロジェクトの定期同期を始める前に共有キャッシュを確認する:

1. `shared_project_state` にそのプロジェクトの行があり、
2. `refreshed_at` が現在時刻から 15 分 (`PEER_FRESH_MAX_AGE_SECS`) 以内で、
3. `written_by` が自分自身ではなく（自分が書いたものを自分の鮮度判定に
   使うと、常に「新鮮」と誤判定して相手の更新を永久に見なくなる）、
4. `included_*` が自分がこのプロジェクトで必要とする範囲を含んでいれば、

→ 自分の Azure DevOps API 呼び出しを省略し、共有キャッシュの該当行を
自分のローカルキャッシュ (`replace_project_cache`) へそのまま採用する。

いずれか一つでも満たさなければ、従来どおり自分で API 同期し、成功したら
結果を共有キャッシュへも書き戻す (`written_by` を自分の名前で upsert)。

15 分という閾値は、定期フル同期の間隔 (12 時間) よりずっと短くし、
「鮮度を大きく落とさずに重複同期だけを避ける」ことを狙って選んだ値。

### 同時実行の安全性

SQLite の `busy_timeout` (5秒) を waypoint 側の `azure_devops.db` と同じ
方針で設定する。書き込みはプロジェクト単位の DELETE→INSERT トランザクション
で、waypoint 側の `replace_project_cache` と同型。

### 既知の制約 (v1 で許容する)

- **`is_mine` は両アプリが同一の Azure DevOps アカウントで認証している前提**
  で扱う。別アカウントの PAT を使っている場合、相手のキャッシュを採用すると
  `is_mine` の判定がずれうる。単一ユーザーが自分用に両アプリを使う想定
  なので許容する
- DevDeck 側のスキーマ・書き込み実装はこの spec のスコープ外
  (`waypoint-devdeck-dedup-sync` セッション / DevDeck リポジトリの
  `feature/shared-azure-cache-schema` ブランチが担当)。本 spec はスキーマと
  dedup 判定ロジックについて waypoint 側からの提案であり、相手側の実装が
  異なるスキーマ・パスを選ぶ場合は追って合わせる

## waypoint 側の変更範囲

- 新規 `src/azure_devops/shared_cache.rs`: 共有キャッシュの読み書きと
  鮮度判定 (`peer_fresh_candidates` / `publish_project`)
- `src/azure_devops/api.rs` の `refresh_project`: 同期前に
  `shared_cache::peer_fresh_candidates` を確認し、新鮮なら採用して API を
  スキップ。そうでなければ従来どおり同期し、成功後に
  `shared_cache::publish_project` で書き戻す
- `docs/spec.md`: 02章スコープ表に新規エントリ、FR-9.18.5 を追加
  （「共有メニュー/設定同期」除外の対象は複数ユーザー間の同期衝突であり、
  本機能は単一ユーザー自身の複数アプリ間の重複回避のため主体が異なる）
