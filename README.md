# waypoint

ホットキーで呼び出す階層ポップアップランチャー（Windows）。

登録した場所へ一手で移動する常駐アプリ。デスクトップのショートカット、エクスプローラーのクイックアクセス、タスクバーのピン留めに分散した導線を、ひとつのポップアップメニューに集約する。

> **状態: 開発初期。** 現時点ではタスクトレイに常駐して終了できるところまでで、メニュー表示は未実装。
> 仕様は [docs/spec.md](docs/spec.md) を参照。

## 特長（計画）

- マウス中ボタンまたはホットキー（既定 `Win+W`）でカーソル位置にメニューを表示
- フォルダ・特殊フォルダ・サブメニュー・区切りの 4 種類の項目
- 新規ウィンドウで開く / 開いているエクスプローラーのフォルダを変更する の切り替え
- 環境変数とユーザー変数によるパス展開
- 設定は単一の JSON。手編集もできる

## 動作環境

- Windows 10 22H2 以降
- ランタイム不要（ネイティブバイナリ）
- x64（ARM64 機ではエミュレーションで動作）

## インストール

GitHub Releases から MSI を手動でダウンロードする代わりに、最新版を自動取得して
インストールするスクリプトが使える。

```powershell
irm https://raw.githubusercontent.com/ntaksh42/waypoint/main/installer/install.ps1 | iex
```

MSI を手元に持っている場合はダブルクリックする。ユーザー単位でインストールするので管理者権限は不要。

```powershell
# サイレントインストール
msiexec /i waypoint-0.1.0-x64.msi /qn
```

| | |
|---|---|
| インストール先 | `%LOCALAPPDATA%\Programs\waypoint` |
| 設定 | `%APPDATA%\waypoint\config.json` |
| アンインストール | 設定 > アプリ から。設定ファイルは残る |

常駐中でも上書きインストールできる（インストーラが自動で終了させる）。
自動起動はインストーラでは設定しない。トレイメニューの
`Start with Windows` で切り替える。

### MSI を自分でビルドする

[WiX 5](https://wixtoolset.org/) が必要。

```powershell
dotnet tool install --global wix --version 5.0.2
.\installer\build.ps1
```

`dist\waypoint-<version>-x64.msi` ができる。バージョンは `Cargo.toml` から
読むので、上げるときは `Cargo.toml` だけ直せばよい。

## ソースからのビルド

```powershell
cargo build --release
```

出力: `target\release\waypoint.exe`

## 開発

```powershell
cargo run                                    # 起動
cargo test                                   # テスト
cargo clippy --all-targets -- -D warnings    # 静的検査
cargo fmt --all                              # 整形
```

リビルド前に実行中の `waypoint` を終了すること（実行中の exe は出力をロックする）。

```powershell
Get-Process waypoint -ErrorAction SilentlyContinue | Stop-Process -Force
```

## ライセンス

MIT
