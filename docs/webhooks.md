# Webhook

外部サービスからの通知を受け取り、通知専用 App としてチャンネルへ投稿します。受信口は `POST /api/webhooks/{id}` の 1 種類です。本文は次の 2 つの形式を受け付けます。

- **Slack 形式**：Slack Incoming Webhook と同じ `text`・`blocks`（Block Kit）・`attachments`
- **Discord 形式**：Discord Webhook と同じ `content`・`embeds`・`components`（Components V2）・`flags`

どちらも、zoo のメッセージ部品（Discord Components V2 と同じ構造。[App・Bot 基盤](apps.md#メッセージ部品)）に変換して表示します。

## 設定

1. チャンネルの設定から Webhook を開き、名前を指定して作成します。チャンネル管理者またはワークスペース管理者が操作できます。専用の投稿 App は自動作成されます。
2. 表示された「Webhook URL」をコピーし、送信元の Slack Incoming Webhook URL、または Discord Webhook URL に設定します。URL は `/api/webhooks/{id}?token=<認証トークン>` の形式です。
3. JSON を POST します。URL 内のトークンで認証するため、本文署名や独自の認証ヘッダーは不要です。

## 送り方

| 項目 | 内容 |
| --- | --- |
| メソッドと URL | `POST /api/webhooks/{id}?token=<認証トークン>`。トークンは `Authorization: Bearer <認証トークン>` でも渡せます（下の Flux の節） |
| `Content-Type` | `application/json` |
| 本文の大きさ | 256 KiB まで。部品に変換したあとの JSON は 64 KB まで（超えた分は末尾の部品から省き、「ほかに N 件の部品を省略しました」と書き足します） |
| 成功 | HTTP 200、本文 `ok` |
| `X-Webhook-Delivery` | UUID を指定すると、同じチャンネルへの再送を重複排除します |

400 になるのは、JSON として読めないとき、JSON がオブジェクトでないとき、本文が 256 KiB を超えるとき、表示するものが何もないとき（`{"text":""}`、`{"blocks":[]}` など）、不正な Delivery ID のときだけです。**形式が正しい Slack・Discord の通知は、知らない項目やブロックが混ざっていても 400 にしません**（下の「部品の一覧」）。

### 形式の見分け方

- `text`・`blocks` のどちらかがあれば **Slack 形式** として読みます。
- それらがなく、`content`・`embeds`・`components`・`flags` のどれかがあれば **Discord 形式** として読みます。`attachments`（Discord では添付ファイルの情報）があっても Discord 形式です。Discord の `attachments` は読みません。
- どれもなく `attachments` だけがあれば、**Slack 形式**（attachments）として読みます。
- `username`・`avatar_url`・`icon_emoji`・`channel`・`tts`・`allowed_mentions` など、投稿者やチャンネルを上書きする項目は無視します。投稿者は常に Webhook の通知専用 App です。

### 本文の代わり（`content`）

部品のある投稿では、チャットに部品だけを表示し、メッセージの本文（`content`）は通知・プッシュ・検索・TUI・古いクライアント用の「代わりの文字」になります。

- **Slack 形式**：`blocks` があれば、`text` を代わりの文字にします（Slack と同じく、`text` は表示しません）。`blocks` がなく `attachments` があれば、`text` を部品の先頭に表示し、代わりの文字は部品の文字から作ります。`text` だけなら、部品を作らずに本文として表示します。
- **Discord 形式**：`content` は部品の先頭に表示します。代わりの文字は部品の文字（見出し・本文・項目・ボタンの行き先など）を改行でつないで作ります。
- 代わりの文字は 40,000 文字までに切り詰めます。本文のメンション（`@name`）で通知は飛びません。

## 例

### Slack：テキストだけ

```bash
curl -X POST "$ZOO_WEBHOOK_URL" -H 'content-type: application/json' \
  -d '{"text":"*デプロイ* が完了しました <https://example.com/deploy/123|詳細>"}'
```

### Slack：Block Kit

```bash
curl -X POST "$ZOO_WEBHOOK_URL" -H 'content-type: application/json' -d @- <<'JSON'
{
  "text": "本番のデプロイが完了しました",
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": "Deploy complete" } },
    {
      "type": "section",
      "text": { "type": "mrkdwn", "text": "*production* は正常です" },
      "fields": [
        { "type": "mrkdwn", "text": "*Cluster*\nsite1" },
        { "type": "mrkdwn", "text": "*Commit*\n`abc1234`" }
      ],
      "accessory": { "type": "image", "image_url": "https://example.com/logo.png", "alt_text": "logo" }
    },
    { "type": "divider" },
    {
      "type": "actions",
      "elements": [
        { "type": "button", "text": { "type": "plain_text", "text": "ログを開く" }, "url": "https://example.com/logs/123" }
      ]
    },
    { "type": "context", "elements": [{ "type": "mrkdwn", "text": "CI · <!date^1700000000^{date_short}|2023-11-14>" }] }
  ]
}
JSON
```

### Slack：attachments（Flux・Alertmanager・Grafana などの旧形式）

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
      "ts": 1700000000,
      "fields": [{ "title": "Cluster", "value": "site1", "short": true }]
    }
  ]
}
```

### Discord：Components V2

```bash
curl -X POST "$ZOO_WEBHOOK_URL" -H 'content-type: application/json' -d @- <<'JSON'
{
  "flags": 32768,
  "components": [
    {
      "type": 17,
      "accent_color": 3066993,
      "components": [
        { "type": 10, "content": "## Deploy complete\n**production** は正常です" },
        {
          "type": 9,
          "components": [{ "type": 10, "content": "Commit `abc1234`\n-# 2 分前" }],
          "accessory": { "type": 11, "media": { "url": "https://example.com/logo.png" } }
        },
        { "type": 14 },
        { "type": 1, "components": [{ "type": 2, "style": 5, "label": "ログを開く", "url": "https://example.com/logs/123" }] }
      ]
    }
  ]
}
JSON
```

### Discord：embeds（旧来の形式）

```json
{
  "content": "デプロイが完了しました",
  "embeds": [
    {
      "title": "Deploy complete",
      "url": "https://example.com/deploy/123",
      "description": "Production is healthy",
      "color": 3066993,
      "fields": [{ "name": "Cluster", "value": "site1", "inline": true }],
      "footer": { "text": "CI" },
      "timestamp": "2026-11-16T00:00:00Z"
    }
  ]
}
```

### 送信元ごとの設定

- **Grafana**：通知先（Contact point）の種類に「Slack」か「Discord」を選び、Webhook URL に zoo の URL を入れます。Slack 型では「Incoming Webhook URL」の欄を使います。
- **Alertmanager**：`slack_configs` の `api_url`、または `discord_configs` の `webhook_url` に zoo の URL を設定します。トークンを URL に入れたくない場合は、`http_config.authorization`（`type: Bearer`、`credentials_file`）を使い、`api_url` にはトークンを含めない URL を設定します。
- **GitHub Actions**：`curl` で上の例のような JSON を送ります。URL は Actions の Secret に入れ、ログに出さないでください。
- **Flux**：下の「Flux の設定と送信エラーのマスク」を参照してください。

## 記法（テキストの書き方）

zoo の本文は MFM（[MFM 対応表](mfm.md)）です。Slack の mrkdwn と Discord の markdown は、受け取ったときに MFM へ変換します。MFM にない記法や、変換表にない記法は、文字のまま表示します。

### Slack mrkdwn（`type: mrkdwn` のテキスト、`text`、`attachments[].text` など）

| Slack の書き方 | zoo での表示 |
| --- | --- |
| `*太字*` | 太字（`**太字**`） |
| `_斜体_` | 斜体（`<i>斜体</i>`） |
| `~取り消し~` | 取り消し線（`~~取り消し~~`） |
| `` `コード` ``、` ```コードブロック``` ` | コード（中の記号は変換しません） |
| `> 引用` | 引用 |
| `<https://example.com>` | リンク（URL をそのまま表示） |
| `<https://example.com\|ラベル>` | `[ラベル](https://example.com)` のリンク |
| `<mailto:a@example.com\|ラベル>` | ラベルの文字 |
| `<@U123>`、`<@U123\|名前>` | `@U123`、`@名前`（文字として表示し、通知は飛びません） |
| `<#C123\|general>` | `#general` |
| `<!here>`、`<!channel>`、`<!everyone>` | `@here` などの文字（通知は飛びません） |
| `<!subteam^S123\|@team>` | `@team` |
| `<!date^1700000000^{date_short}\|代わり>` | 代わりの文字 |
| `:smile:` | 絵文字 |
| `&amp;`、`&lt;`、`&gt;` | `&`、`<`、`>` |
| MFM として解釈される並び（`$[`、`<b>` など） | 文字のまま表示（間に見えない区切りを入れて無効にします） |

- `plain_text` のテキストは記法を解釈せず、`:smile:` も含めて文字のまま表示します（`emoji` は無視します）。`verbatim` も無視します（zoo は URL やチャンネル名を自動でリンクにしません）。
- `attachments` の `mrkdwn_in` に含めた項目だけを mrkdwn として読みます。`mrkdwn_in` がなければ、`text`・`pretext`・`fields` を mrkdwn として読みます（Slack の既定と同じ）。
- `rich_text` ブロックは書式（太字・斜体・取り消し・コード）、リスト（番号付き・箇条書き・字下げ）、引用、コードブロック、リンク、絵文字、メンション、日時（`$[unixtime]` で、見る人のタイムゾーンで表示）を変換します。下線と強調表示は表示しません。
- `markdown` ブロック（標準の markdown）は、そのまま MFM として表示します。タスクリスト（`- [ ]`・`- [x]`）は ☐・☑ に、`---` は横線に、`-`・`*` の箇条書きは「・」に変えます。表（GFM）は文字のまま表示します。

### Discord markdown（`content`、`text_display`、embeds の `title`・`description`・`fields`）

| Discord の書き方 | zoo での表示 |
| --- | --- |
| `**太字**`、`*斜体*`、`~~取り消し~~` | そのまま（MFM と同じ） |
| `__下線__` | 太字（MFM に下線がないため） |
| `# 見出し`〜`### 見出し` | 見出し |
| `-# 小さい字` | 小さい字（`<small>`） |
| `\|\|ネタバレ\|\|` | ぼかし（`$[blur …]`。触れると表示） |
| `> 引用`、`>>> 以降すべて引用` | 引用 |
| `- 箇条書き`、`1. 番号付き` | 文字のまま（行頭の記号を表示） |
| `[ラベル](https://example.com)` | リンク |
| `` `コード` ``、` ```コードブロック``` ` | コード |
| `<t:1700000000:R>` など | 日時（`$[unixtime]`。表示の書式は見る人の端末に合わせます） |
| `<@123>`、`<@!123>`、`<@&123>`、`<#123>` | `@123`・`#123` の文字（通知は飛びません） |
| `<:name:123>`、`<a:name:123>` | `:name:`（登録済みの絵文字なら絵文字） |

## 部品の一覧

> 部品の描画は、クライアントの対応（Zli-UoA/zoo2#600）を出したあとの表示です。それまでの Web・Android・デスクトップでは、部品のある投稿は本文（部品から作った代わりの文字）だけを表示します。

受け取った部品は、どれもチャットに表示します。**Webhook には押したときに受け取るアプリがないので、ボタン（リンクボタンを除く）・select・入力欄・ピッカーは、表示はしますが押せない状態にします。** リンクボタンと動画は普通に使えます。

### Slack Block Kit

| ブロック | zoo での表示 |
| --- | --- |
| `section` | 本文。`fields` は 1 行ずつ（2 列にはしません）。`accessory` が画像なら横にサムネイル、ボタンや select なら横に置きます |
| `header` | 見出し（`level` 1〜4。既定は 3） |
| `divider` | 区切り線 |
| `context` | 小さい字。画像は代わりの文字（`alt_text`） |
| `actions`、`context_actions` | ボタンは 5 個ずつの行、select などは 1 行に 1 つ。`feedback_buttons` は 2 つのボタン、`icon_button` はボタン |
| `image` | 画像（続く画像は 1 つのギャラリーにまとめます。`title` は太字の見出し） |
| `rich_text`、`markdown` | 本文（上の記法を参照） |
| `input` | 見出し付きの入力欄（押せない状態） |
| `video` | 動画のカード（サムネイル・タイトル・再生のリンク） |
| `table`、`data_table` | 表（`data_table` は見出しの行・並べ替え・ページ送りあり。ボタンのセルは押せない状態） |
| `alert` | 色付きのお知らせ（`level`：info・warning・error・success） |
| `card`、`carousel` | カード（画像・タイトル・本文・ボタン）。`carousel` は横に並べます |
| `container` | 枠（見出し・折りたたみ付き） |
| `data_visualization` | グラフ（円・棒・面・折れ線） |
| `layout` | 格子状の配置（狭い画面では縦に積みます） |
| `plan`、`task_card` | 作業の一覧と状態 |
| `file` | ファイル名だけ（Slack 内のファイルは取得しません） |

select の選択肢は先頭 25 件を出します（Slack は 100 件まで送れますが、部品の select は 25 件までです。Webhook の select は押せない状態で出すので、選べないことは変わりません）。

要素：`button`（`primary`・`danger` の色、`url` があればリンクボタン、`confirm`）、`static_select`・`external_select`・`users_select`・`conversations_select`・`channels_select`（それぞれ `multi_` も）、`overflow`、`datepicker`・`timepicker`・`datetimepicker`、`checkboxes`・`radio_buttons`、`plain_text_input`・`number_input`・`email_text_input`・`url_text_input`・`rich_text_input`、`file_input`、`image`、`workflow_button` に対応します。

### Slack attachments（旧形式）

`pretext` は枠の前に、それ以外は `color` の色の枠にまとめます。`author_name`（`author_link`）・`title`（`title_link`）・`text`・`fields`・`thumb_url`（横にサムネイル）・`image_url`（画像）・`footer`・`ts`（日時）・`blocks` を表示します。表示するものがなければ `fallback` を表示します。リンクの展開（`from_url`・`is_app_unfurl`）と、旧来の `actions` は表示しません。最大 100 件です。

### Discord

| 部品（type） | zoo での表示 |
| --- | --- |
| Container（17） | 色付きの枠（`accent_color`、`spoiler` はぼかし） |
| Text Display（10） | 本文（上の記法を参照） |
| Section（9） | 本文の横にサムネイルかボタン |
| Thumbnail（11）、Media Gallery（12） | 画像（`spoiler` はぼかし） |
| File（13） | ファイル（https の URL だけ。`attachment://` は使えません） |
| Separator（14） | 区切り線か余白（`divider`、`spacing`） |
| Action Row（1）、Button（2） | ボタン（リンクボタン以外は押せない状態） |
| String / User / Role / Mentionable / Channel Select（3、5〜8） | select（押せない状態） |
| Label（18）、Text Input（4）、File Upload（19）、Radio Group（21）、Checkbox Group（22）、Checkbox（23） | 見出し付きの入力欄（押せない状態） |
| embeds | 色付きの枠（author・title・url・description・fields・thumbnail・image・footer・timestamp） |

### 知らない部品

zoo がまだ知らないブロックや type（Slack や Discord が今後足すもの）は、捨てずに「未対応の部品」として、中の文字（`text`・`label`・`title` など）を拾える分だけ表示します。配置の決まりに合わない部品（container の中の container など）も、同じく文字だけを表示します。

### 上限

| 項目 | 上限 |
| --- | --- |
| 部品の数（入れ子を含む） | 500 |
| 本文の文字の合計 | 40,000 |
| 表のセルの文字の合計 | 20,000 |
| ボタン | 1 行に 5 個 |
| 画像のギャラリー | 10 枚 |
| `blocks`、`attachments` | それぞれ 100 件 |

画像・動画・リンクの URL は `https://` のものだけを使います。それ以外の URL のリンクは文字として表示し、画像は表示しません。画像はプロキシを通して表示し、見る人の IP アドレスを画像の置き場所へ渡しません。

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

## 認証・再送・管理

Webhook URL 全体が認証情報です。設定画面ではマスクして表示し、管理 API は `Cache-Control: no-store` を返します。管理 API の `path` に認証トークン付きの URL を返します。独立した `secret`・`generic_path`・`slack_path` は返しません。

「Webhook URLを再発行」すると認証トークンが変わり、旧 URL は無効になります。送信元も更新してください。管理 API は既存の `POST /api/channels/{channel_id}/webhooks/{id}/rotate-secret` を使用します。

`X-Webhook-Delivery` に UUID を指定すると同一チャンネルへの再送を重複排除します。省略時はリクエストごとに新規投稿となり、送信元の再送によって重複する場合があります。不正な Delivery ID は400エラーになります。

「削除」では受信口を無効化し、不要になったチャンネルへの参加を解除します。投稿済みメッセージと投稿 App は履歴のため保持します。アーカイブ・削除済みチャンネルには投稿できません。

## 部品の形式への移行（2026-11）

これまで Webhook の投稿はカード（`messages.card`）として保存していました。今後は部品（`messages.components`）として保存します。送信元の設定変更は不要です。これまで 400 にしていた `blocks` も受け付けます。保存済みのカードは `zoo_components_backfill` で部品に移します（[App・Bot 基盤](apps.md#メッセージ部品)の「移行」）。

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
