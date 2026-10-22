# GitHub Webhook

GitHub の Push・Pull Request・Issue・Release・GitHub Actions (`workflow_run`) をチャンネルに投稿し、カードで表示します。

## チャンネルから作成する

1. チャンネル右上の管理メニュー（⋯）→「Webhook設定」を開きます。
2. Webhook名を入力して「作成」を押します。専用の投稿ユーザーが自動作成されます。
3. 表示された **URL** と **Secret** をそれぞれコピーボタンでコピーします。Secretは通常伏せ字で、目のボタンで表示できます。閉じた後も同じ画面で再確認できます。
4. GitHub リポジトリの Settings → Webhooks → Add webhook で次を設定します。
   - Payload URL: コピーしたURL
   - Content type: `application/json`
   - Secret: コピーしたSecret
   - SSL verification: 有効
   - Events: Pushes、Pull requests、Issues、Releases、Workflow runs から必要なものを選択
5. GitHub の Recent Deliveries で受信結果を確認します。初回の `ping` は200を返します。

Webhookはチャンネルごとに複数作成できます。管理できるのはチャンネル作成者とワークスペース管理者です。一般メンバーはURL・Secretの取得もできません。DMでは利用できません。アーカイブ中は新規作成・投稿できませんが、既存Webhookの確認・Secret再発行・削除は可能です。

「Secretを再発行」では確認後に旧Secretが無効になります。GitHub側のSecretも更新してください。URLは変わりません。「削除」では確認後に受信口を無効化します。投稿済みメッセージと投稿ユーザーは履歴保持のため残し、チャンネルへの参加は解除します。

## サーバーへの反映

DBマイグレーション `20261023010000_create_channel_webhooks.sql` を適用してから、バックエンドとフロントを更新してください。既存の手順では `make -C infra migrate` で適用できます。

チャンネルから作成するWebhookには環境変数の追加は不要です。URLのホストはフロントのAPI接続先設定（`VITE_API_BASE_URL`、同一オリジンなら現在のホスト）を使うため、GitHubから到達可能な公開URLを設定してください。

Secretは再表示に対応するためDBに保存します。管理APIのレスポンスは `Cache-Control: no-store` を付与し、Electricの同期対象には含めません。DB・バックアップへのアクセスも適切に管理してください。

## 既存の環境変数による受信口

従来の `POST /api/webhooks/github` も引き続き利用できます。こちらは以下の3つを設定し、バックエンドを再起動してください。チャンネルの設定画面には表示されません。

```dotenv
GITHUB_WEBHOOK_SECRET=<十分に長いランダムなシークレット>
GITHUB_WEBHOOK_CHANNEL_ID=<投稿先チャンネルのUUID>
GITHUB_WEBHOOK_USER_ID=<投稿に使う既存ユーザーのUUID>
```

専用の投稿ユーザーを作成し、非公開チャンネルではそのユーザーをメンバーに追加してください。3つすべてが設定されていない場合、従来の受信口は404を返します。

## 受信と表示

署名は生の本文に対する `X-Hub-Signature-256` (HMAC-SHA256) を定時間比較で検証します。[GitHub公式仕様](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries)に従っています。本文にはAxumの既定の2 MiB制限が適用されます。

対応イベントの投稿成功は200、未対応イベントは204です。同じ配信IDの再送では投稿を重複させません。同時に届いた場合は片方が競合エラーになる可能性がありますが、再送時は200を返します。Webhook本文の `@channel` などでは通知を発生させません。

カードにはリポジトリ、イベント、タイトル、投稿元ユーザー、詳細へのHTTPSリンクを表示します。Pushは最大10コミット、本文は最大2,000文字です。外部本文はプレーンテキストで表示します。

カードは `zoo:github:v1` とJSONをメッセージ本文に保存し、タイムラインとスレッドで表示します。この形式は表示用であり、GitHub由来を保証する認証バッジではありません。通常ユーザーも同じ形式の本文を投稿できます。
