# App・Bot 基盤の導入前に確認すること

この変更は GitHub 専用 Webhook の削除を含みます。既存の送信元を切り替える前に本番へ適用すると、GitHub 通知が停止します。以下の移行を先に完了してください。

## GitHub 通知の移行

削除対象は `/api/webhooks/github`、`/api/webhooks/github/{id}` と `GITHUB_WEBHOOK_*` の設定です。GitHub 固有の変換機能は Zoo 本体へ戻しません。

1. GitHub イベントを受信・検証する外部 Bot アプリを用意し、Zoo の App として登録します。必要な scope と投稿先チャンネルへの参加を管理者が承認します。
2. Bot アプリ側の受信 URL と GitHub の署名 Secret を設定し、GitHub 側の Webhook の送信先をその URL へ変更します。Zoo の共通 Webhook URL は GitHub の生ペイロードを受け付けません。
3. GitHub のテスト配信と実際の対象イベントで、Bot アプリ経由の投稿を確認します。旧通知と重複しないことも確認します。
4. 旧 Zoo 側の GitHub 環境変数を除去し、このバージョンをデプロイします。Bot アプリの準備・切り替えが未完了なら、本番への適用を延期してください。

共通 Webhook の URL・Secret・重複排除、既存の投稿者と通知カードは保持します。共通 Webhook の設定手順は [webhooks.md](../webhooks.md)、Bot アプリの設定は [apps.md](../apps.md) を参照してください。

## 追加マイグレーション

`20261029000000_preserve_terminal_events_and_dispatch_fairly.sql` まで適用してからバックエンドを更新します。既存マイグレーションの書き換えは不要です。この更新は、既存キューに残っている metadata・リンク通知も現在の権限に応じた形式へ修正します。

App を無効化・削除すると、その App が発行した公開ファイル URL は失効します。再インストールしても古い URL は復活しません。チャンネル内のファイル履歴は従来の閲覧権限に従って残ります。
