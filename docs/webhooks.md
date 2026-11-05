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

`{"text":"Hello"}` のようなテキストのみの通知も受け付けます。Flux は下記のヘッダー認証の設定を使用してください。Flux が送信する `good`・`danger` の色をそのまま表示します。

## Flux の設定と送信エラーのマスク

Flux notification-controller v1.9.4 では、Slack Provider の再送が失敗すると、Secret の `address` に含めた query トークンがログや Kubernetes Event に残る場合があります。Flux には設定画面のトークン付き URL をそのまま渡さず、URL とトークンを分けて設定してください。

同じ受信口は `Authorization: Bearer <認証トークン>` にも対応します。設定画面でコピーした URL の `?token=` より前を `address`、`token` の値を Secret の `token` に設定します。

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: zoo-webhook
  namespace: flux-system
type: Opaque
stringData:
  address: https://zli.chat/api/webhooks/WEBHOOK_ID
  token: WEBHOOK_TOKEN
---
apiVersion: notification.toolkit.fluxcd.io/v1beta3
kind: Provider
metadata:
  name: zoo
  namespace: flux-system
spec:
  type: slack
  secretRef:
    name: zoo-webhook
```

`WEBHOOK_ID` と `WEBHOOK_TOKEN` は設定画面の値に置き換えてください。実際の Secret は平文で Git に登録せず、既存の Secret 管理方法で適用します。通知先の Alert はこの Provider を参照させます。

v1.9.4 の [Slack 送信処理](https://github.com/fluxcd/notification-controller/blob/v1.9.4/internal/notifier/slack.go) は Secret.token を Authorization ヘッダーに設定し、[失敗時の処理](https://github.com/fluxcd/notification-controller/blob/v1.9.4/internal/server/event_handlers.go) はその値をマスクします。この設定では送信先 URL に認証情報が含まれません。別バージョンや独自 relay を使用する場合も、トークンを URL に含めずヘッダーで送信し、エラーログ・Event・プロキシのアクセスログにヘッダーを記録しないことを確認してください。実クラスタでの失敗ログは本 PR のテスト対象外です。

Authorization ヘッダーがある場合はヘッダーを優先し、不正なヘッダーから query トークンへのフォールバックは行いません。URL の再発行ではヘッダーに使用する旧トークンも無効になります。

## 対応する表示項目

- `text` と、最大100件の `attachments` を受け付けます。添付カードは1件のメッセージ内に順番に表示します。
- 添付カードの `title`、`title_link`、`text`、`fallback`、`pretext`、`author_name`、`footer`、`fields[].title/value` を表示します。`fallback` はタイトルも本文もない場合に使用します。タイトル省略時は `Webhook` と表示します。`short` による列配置は行いません。
- `color` は `good`（緑）、`warning`（黄）、`danger`（赤）、6桁の16進色コード（`#` は省略可）を受け付けます。省略時はブランド色です。不正な色は400エラーになります。
- Block Kit（`blocks`）、操作ボタン、画像、アイコン、送信者名・チャンネルの上書きには対応しません。未知の表示項目は無視します。対応する本文がない通知は400エラーになります。
- HTML・Slack mrkdwn は解釈せずプレーンテキストとして表示します。リンクは `title_link` の HTTPS URL のみです。

読み上げ・管理画面・旧クライアント向けの `content` には、各カードのタイトルと本文を順番に連結し、40,000文字以内に収めた要約を保存します。タイトルが省略されたカードは本文だけを使用します。要約を切り詰めても、保存するカード本体は変更しません。

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

既存の投稿 App・投稿履歴・保存済みカードは保持します。同時切り替えや無停止移行は保証しません。送信元の運用担当者は以下の順序で切り替えてください。

1. デプロイ前に全送信元を洗い出し、旧 URL・署名 Secret・JSON 変換設定を Secret 管理先に保管します。zoo2-infra の alert-relay が稼働している場合、旧カードJSONと署名を送るため、URLだけでなく変換処理と認証も更新する準備をします。relay を外して Flux の Slack Provider に切り替える場合は上記の設定を用意します。
2. 送信処理を一時停止し、未送信通知を送信元のキュー等へ保存します。停止中に発生するイベントを保存・再送できない送信元は通知欠落を防げないため、欠落を許容できる保守時間の合意が必要です。実環境の送信元・キュー・許容時間はこの PR では確認していません。
3. Zooをデプロイし、管理画面から新 URL を取得して、送信先・JSON・認証設定を更新します。Flux は URL と token を分けます。成功・失敗のテスト通知で、色と保存済み本文を確認します。
4. 送信を再開し、保存した未送信通知を再送します。以前に配信済みの通知には同じ Delivery ID を使用します。
5. ロールバックする場合は送信を再び停止し、Zoo を旧バージョンへ戻してから、保管した旧 URL・署名 Secret・旧 JSON 変換設定を送信元へ戻します。旧形式でのテスト通知を確認した後に送信と再送を再開します。切り替え完了まで Webhook の削除や URL の再発行は行いません。

新旧形式を併用する移行期間は設けません。上記の停止・再送を用意できることをデプロイ前に確認してください。

## GitHub 連携

GitHub の生ペイロードは Slack 形式ではありません。GitHub との連携は外部 Bot アプリで受信・検証してから投稿してください（[App・Bot 基盤](apps.md)）。

`POST /api/webhooks/github` と `POST /api/webhooks/github/{id}`、`GITHUB_WEBHOOK_*` の設定は削除済みです。GitHub 専用機能からの移行は [リリースノート](releases/app-platform.md) を参照してください。
