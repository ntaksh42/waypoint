# Browser Tab Bridge

`t ` で Chrome / Edge の現在開いているタブを検索し、Enter でそのタブを前面化します。
タブのタイトルと URL は waypoint のメモリだけに保持され、設定・履歴ファイルには保存しません。

## 導入

通常の MSI インストールでは、初回完了後に `setup.html` が自動で開く。表示した案内に従って、使うブラウザだけへ拡張を読み込む。

案内を閉じた後に再表示する場合は、インストール先の `browser-tabs\setup.html` を開く。

1. Chrome で `chrome://extensions`、Edge で `edge://extensions` を開き、開発者モードを有効にする。
2. **Load unpacked** で、このフォルダの `extension` をそれぞれ読み込む。
3. waypoint を起動する。Native Messaging host の登録は起動時に自動で行われる。

Chrome / Edge は、Windows アプリがローカル拡張を無確認でインストールすることを許可していない。そのため拡張を読み込む操作だけはブラウザごとに最初の一度必要で、以後は waypoint を起動するだけでよい。

拡張の ID は `fllmalpfkkdholloicheiplekihkkjeo` で固定している。表示が異なる場合は、拡張を一度削除して `extension` フォルダを再度読み込む。

ブラウザまたは拡張が起動していない場合、`t ` は候補を返さない。
