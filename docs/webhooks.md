# Webhook

外部サービスから Slack Incoming Webhook 形式の JSON を受け取り、通知専用 App としてチャンネルへ投稿します。受信口は `POST /api/webhooks/{id}` の1種類です。

## 設定

1. チャンネルの設定から Webhook を開き、名前を指定して作成します。チャンネル管理者またはワークスペース管理者が操作できます。専用の投稿 App は自動作成されます。
2. 表示された「Webhook URL」をコピーし、送信元の Slack Incoming Webhook URL に設定します。URL は `/api/webhooks/{id}?token=<認証トークン>` の形式です。
3. JSON を POST します。URL 内のトークンで認証するため、本文署名や独自の認証ヘッダーは不要です。成功時は HTTP 200 と本文 `ok` を返します。

```json
{
  "text": "配備結果",
  "attachments": [
    {
      "title": "Deploy complete",
      "title_link": "https://example.com/deploy/123",
      "text": "Production is healthy",
      "color": "good",
      "author_name": "CI",
      "footer": "production",
      "fields": [{ "title": "Cluster", "value": "site1", "short": true }]
    }
  ]
}
```

`{"text":"Hello"}` のようなテキストのみの通知も受け付けます。Flux は Provider の `type: slack` を使用し、Secret の `address` にこの URL を設定します。Flux が送信する `good`・`danger` の色をそのまま表示します。

## 対応する表示項目

- `text` と、最大100件の `attachments` を受け付けます。添付カードは1件のメッセージ内に順番に表示します。
- 添付カードの `title`、`title_link`、`text`、`fallback`、`pretext`、`author_name`、`footer`、`fields[].title/value` を表示します。`fallback` はタイトルも本文もない場合に使用します。タイトル省略時は `Webhook` と表示します。`short` による列配置は行いません。
- `color` は `good`（緑）、`warning`（黄）、`danger`（赤）、6桁の16進色コード（`#` は省略可）を受け付けます。省略時はブランド色です。不正な色は400エラーになります。
- Block Kit（`blocks`）、操作ボタン、画像、アイコン、送信者名・チャンネルの上書きには対応しません。未知の表示項目は無視します。対応する本文がない通知は400エラーになります。
- HTML・Slack mrkdwn は解釈せずプレーンテキストとして表示します。リンクは `title_link` の HTTPS URL のみです。

本文上限は2 MiBです。変換後の各カードのタイトルは500文字、説明は4,000文字、リンク・著者・フッターなどの各項目は2,000文字までです。`pretext`・`text`・`fields` を連結した説明にも4,000文字の制限を適用します。各添付の `fields` は最大100件です。

## 認証・再送・管理

Webhook URL 全体が認証情報です。設定画面ではマスクして表示し、管理 API は `Cache-Control: no-store` を返します。管理 API の `path` に認証トークン付きの URL を返します。独立した `secret`・`generic_path`・`slack_path` は返しません。

「Webhook URLを再発行」すると認証トークンが変わり、旧 URL は無効になります。送信元も更新してください。管理 API は既存の `POST /api/channels/{channel_id}/webhooks/{id}/rotate-secret` を使用します。

`X-Webhook-Delivery` に UUID を指定すると同一チャンネルへの再送を重複排除します。省略時はリクエストごとに新規投稿となり、送信元の再送によって重複する場合があります。不正な Delivery ID は400エラーになります。

「削除」では受信口を無効化し、不要になったチャンネルへの参加を解除します。投稿済みメッセージと投稿 App は履歴のため保持します。アーカイブ・削除済みチャンネルには投稿できません。

## 旧共通カード形式からの移行

Webhook の受信仕様を Slack 互換形式に置き換えます。既存の Webhook 設定は再作成せず使用できますが、送信元の設定変更が必要です。

1. チャンネル設定からトークン付きの Webhook URL をコピーし、送信元の URL を更新します。
2. 旧 `title`・`description`・`url` 等を直接送る形式から、上記の `text`・`attachments` 形式へ変更します。例えば `title` は `attachments[].title`、`description` は `attachments[].text`、`url` は `attachments[].title_link` に移します。
3. 本文署名の `X-Webhook-Signature-256` は不要です。トークンのない URL と旧共通カード形式では受信できません。
4. 再送を重複排除する送信元は、引き続き `X-Webhook-Delivery` に同じ UUID を指定してください。

既存の投稿 App・投稿履歴・保存済みカードは保持します。デプロイ時には送信元の切り替えも行ってください。

## GitHub 連携

GitHub の生ペイロードは Slack 形式ではありません。GitHub との連携は外部 Bot アプリで受信・検証してから投稿してください（[App・Bot 基盤](apps.md)）。

`POST /api/webhooks/github` と `POST /api/webhooks/github/{id}`、`GITHUB_WEBHOOK_*` の設定は削除済みです。GitHub 専用機能からの移行は [リリースノート](releases/app-platform.md) を参照してください。
