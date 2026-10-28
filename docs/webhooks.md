# 共通 Webhook

外部サービスから共通カード形式の JSON を受け取り、通知専用 App としてチャンネルへ投稿します。GitHub 固有の受信・ペイロード変換は削除しました。GitHub との連携は外部の Bot アプリとして実装してください（[App・Bot 基盤](apps.md)）。

## 設定

1. チャンネルの設定から Webhook を開き、名前を指定して作成します。チャンネル管理者またはワークスペース管理者が操作できます。
2. 表示された「共通Webhook URL」と Secret を送信元に設定します。専用の投稿 App は自動作成されます。
3. 生の本文に対する HMAC-SHA256 を Secret で計算し、`X-Webhook-Signature-256: sha256=<16進数>` に設定します。`X-Webhook-Delivery` には再送でも同じ UUID を指定します。
4. `POST /api/webhooks/{id}` に送信します。同じチャンネルへの同じ Delivery ID の再送は重複投稿しません。本文の上限は 2 MiB です。

```json
{
  "title": "Deploy complete",
  "source": "Deploy system",
  "description": "Production is healthy",
  "url": "https://example.com/deploy/123",
  "context": "production",
  "author": "CI",
  "badge": "success"
}
```

必須項目は `title` のみです。タイトルは500文字、説明は4,000文字、その他の各項目は2,000文字までです。URLは省略・空文字、またはHTTPSを指定できます。HTMLやMarkdownは実行せず、プレーンテキストとして表示します。

「Secretを再発行」では旧 Secret が無効になります。送信元も更新してください。URL は変わりません。「削除」では受信口を無効化し、不要になったチャンネルへの参加を解除します。投稿済みメッセージと投稿 App は履歴のため保持します。アーカイブ・削除済みチャンネルには投稿できません。

Secret は共有キャッシュへ保存せず、Webhook 管理 API は `Cache-Control: no-store` を返します。管理 API の `path` は共通 Webhook の URL を返し、既存クライアント向けに `generic_path` も同じ URL を返します。

## GitHub 専用機能からの移行

`POST /api/webhooks/github` と `POST /api/webhooks/github/{id}`、`GITHUB_WEBHOOK_SECRET` / `GITHUB_WEBHOOK_CHANNEL_ID` / `GITHUB_WEBHOOK_USER_ID` の設定は削除しました。GitHub の署名・イベント形式を Zoo 本体で解釈しません。

既存の共通 Webhook URL・Secret・投稿者・投稿履歴は保持します。GitHub 側の送信先は Bot アプリの受信先へ変更してください。既存の GitHub 通知カードはそのまま表示できます。
