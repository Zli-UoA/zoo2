# Zoo App・Bot基盤

ZooはAppの認証、操作API、イベント、対話画面を提供します。Botプログラムは外部で実行します。1つのZoo環境が1ワークスペースです。既存のSlack SDKやBlock Kitのワイヤー互換ではありません。共通の業務処理層を使っているため、互換アダプターは別途追加できます。

API定義は [apps.openapi.json](apps.openapi.json)、イベント形式は [app-events.schema.json](app-events.schema.json) を参照してください。

## 最初のBotを動かす

1. マイグレーションを適用してバックエンドとフロントを更新します。`make migrate` が新しいテーブル・トリガーを作成します。バックエンド本体がHTTPイベント配信ワーカーも起動します。
2. 左レールの「App・連携」からAppを作成します。メンバーが開発者となり、設定をJSONで登録します。
3. 「承認を申請」します。管理者は表示される権限と接続先を確認し、「承認してインストール」を選びます。承認は特定の設定リビジョンに対して行われます。
4. Appを使うチャンネルでApp詳細を開き、「このチャンネルに追加」します。非公開チャンネルには、そのチャンネルへのアクセスと管理権限が必要です。
5. 開発者が「トークンを発行」で認証情報をコピーし、外部実行環境へ保存します。access tokenとrefresh tokenはハッシュだけをDBへ保存し、発行時にだけ平文を返します。
6. Node.js 22.18以降で `node --experimental-strip-types examples/echo-bot/bot.ts` を実行します。`ZOO_URL`、`ZOO_ACCESS_TOKEN`、`ZOO_REFRESH_TOKEN` を環境変数で渡します。Botのユーザー名でメンションするとスレッド返信します。

設定例:

```json
{
  "scopes": ["messages:read", "messages:write", "dm:write", "ui:write"],
  "events": ["app.mentioned"],
  "transport": "websocket",
  "dm_enabled": true
}
```

ユーザー名はApp登録時の `bot_username` で指定でき、省略すると名前から一意なハンドルを生成します。名前、説明、アイコンはAppプロフィールです。人の一覧・招待候補にはBotを出しません。参加者パネルではAppを別グループにし、会話対応Botだけをメンション・DM候補に出します。Appアイコンは任意のHTTPS URLを指定でき、本番では画像配信元のオリジンを `CSP_APP_IMAGE_SRC` に追加します。未設定や取得失敗時はBotアイコンを表示します。

## 権限・認証

App設定の `scopes` は [OpenAPI の Manifest 定義](apps.openapi.json) に列挙しています。各イベントの必要な scope は [対応表](slack-event-coverage.md) で確認できます。

管理者の承認、トークンのscope、操作主体の権限、対象への現在のアクセスをすべて満たす必要があります。Botは参加しているチャンネルだけを読み書きできます。ユーザー代理操作も本人の権限を超えません。scopeのないAPI呼び出しは403、閲覧できないチャンネルは404です。インストール単位で毎分120リクエストに制限し、超過時は429と `Retry-After: 60` を返します。

アクセストークンは1時間、更新トークンは発行から30日間です。更新は単回使用で、旧アクセストークンも失効します。外部プログラムは更新後の認証情報を安全な永続ストアへ保存してください。SDKの `saveCredentials` で更新を受け取れます。

設定の変更は新しいリビジョンを作り、再承認までAppを停止します。名前・説明・アイコンだけの変更では再承認しません。再承認時は古いトークンを失効させます。「無効化」はトークンを失効させ、Botをチャンネルから外します。投稿履歴とプロフィールは保持します。API・継続中のShape・WebSocketも停止します。チャンネル退出時にも配信とShapeのアクセスを再確認します。

開発者・管理者は `/api/apps/{id}/records/credentials` で認証情報のID・scope・状態を確認し、`DELETE /api/apps/{id}/credentials/{credential}` で個別失効できます。秘密値は一覧へ返しません。設定履歴は `records/versions`、監査履歴は `records/audit` です。

### ユーザー代理操作

Appの `redirect_uris` にHTTPSのコールバックURLを登録し、管理者の承認を受けます。ブラウザで次の画面へ誘導します。

```
/apps-authorize?app_id=...&redirect_uri=...&scope=messages%3Awrite&state=...&code_challenge=...&code_challenge_method=S256
```

画面は要求権限と開発者を表示し、本人が許可するscopeを選択します。認可コードは5分間・単回使用で、登録済みの正確なredirect URIとS256 PKCEに紐づきます。本人の画面から許可を取り消せます。OAuth token/revoke APIはJSONを受け付けます。Appクライアントはコールバックで `state` を検証してください。

## API・イベント

外部App用の操作APIは `/api/apps/v1` です。メッセージ、リアクション、チャンネル操作、DMは人向けAPIと共通の処理を使います。書き込みは既存の `{data,txid}` を返し、プラットフォーム設定・対話APIはJSONを直接返します。メッセージ作成のUUID `id` は冪等キーです。同じ投稿内容の再送は既存メッセージを返し、別の内容・別の投稿者への再利用は409です。

読み取りは `/api/apps/v1/shapes/{shape}` に統一します。対応するshapeは `messages`、`message_reactions`、`channels`、`channel_users`、`users` です。会話データには `channel_id` を指定します。Electricの `offset`、`live`、`subset__*` を使用でき、SDKの `shape()` は認証付きResponseを返します。これはチャットイベント配信とは別の、状態の読み取り・同期APIです。

イベントは業務データと同じDBトランザクションで記録されます。投稿・編集・削除、メンション、リアクション、参加・退出、チャンネル変更、ファイル作成、対話操作、Appのインストール・無効化を扱います。`events` に型を登録します。Bot由来の会話イベントは既定で配信せず、必要な場合だけ `include_bot_events` を有効にします。

配信は最低1回で、厳密な順序は保証しません。イベントIDで重複を排除してください。SDKは同じクライアント内で非同期 `EventStore.has()` を含む処理全体をイベント ID ごとに共有し、同時処理と直近1,000件を重複排除します。再起動をまたぐ排除には `EventStore` を実装し、副作用も冪等にしてください。ACK期限は3秒、失敗時は指数バックオフ（最大1時間）で最大24時間再送します。配信履歴は30日保持し、開発者・管理者が確認・再送できます。

### WebSocket

`transport: "websocket"` を指定します。Botトークンで `/api/apps/v1/socket-ticket` を呼び、30秒・単回使用のチケットで返却先のWebSocketへ接続します。長期トークンはURLに含めません。配信の `delivery_id` と `ack_token` を、`{"ack":"...","ack_token":"..."}` として返します。ACKはその試行に紐づきます。

無効化イベントは接続中のSocketへ最後に配信して切断します。無効化後の新規接続は許可しないため、切断済みSocketへの無効化通知は受信できません。状態の最終確認には管理画面・監査履歴を使ってください。

### HTTP

`transport: "http"` と公開HTTPSの `event_url` を指定します。「接続情報」で署名Secretを取得し、受信側で生本文を検証します。`x-zoo-timestamp` と `x-zoo-signature` は `v1:<timestamp>:<raw body>` に対するHMAC-SHA256です。SDKの `verifySignature` は5分の許容時間も検証します。3秒以内の2xxがACKです。受信例は `examples/echo-bot/http.ts` です。

宛先のDNSを各試行で解決し、公開IPだけを許可します。解決結果をリクエストへ固定し、リダイレクトは追いません。HTTPとWebSocketで配信履歴を共用し、インストールごとに一方を選びます。署名Secretは配信のためDBに保存するので、DB・バックアップへのアクセスを管理してください。

## 対話画面・ファイル・リンク

`commands` に登録したコマンドはチャンネルで `/コマンド 引数` として実行できます。同名コマンドが複数のAppにある場合はApp詳細から選択します。`shortcuts` はApp詳細から実行します。`message_shortcuts` はメッセージのApp操作ボタンから実行でき、対象メッセージのIDをイベントへ渡します。メッセージ用ショートカットには `messages:read` が必要です。

対話画面は `text`、`button`、`input`、`select` の宣言型ブロックで構成し、任意HTML・JavaScriptを実行しません。メッセージの `blocks` APIと、Appホーム・モーダル・本人だけに表示する一時メッセージの `surfaces` APIで共用します。操作はApp・本人・元の画面／メッセージ・チャンネルに紐づきます。

モーダルの起動には対話イベントで受け取る `trigger_id` が必要です。3秒・単回使用で、対象ユーザーとチャンネルを変更できません。イベントの `response_path` は15分・単回使用の非同期応答用で、元のユーザー・チャンネルへの一時メッセージだけを作成できます。これらの値は秘密情報として扱ってください。個人向けの画面は共有Shape・オフラインキャッシュへ保存しません。

ファイルはS3互換ストレージを使用します。`APP_S3_ENDPOINT`（HTTPS origin）、`APP_S3_BUCKET`、`APP_S3_REGION`、`APP_S3_ACCESS_KEY_ID`、`APP_S3_SECRET_ACCESS_KEY` を設定します。バケットを非公開にし、ブラウザからPUTする場合はZoo origin・PUT・Content-Type・If-None-MatchをCORSで許可します。アップロードは5分、ダウンロードは60秒の署名付きURLです。25 MiBまでで、サイズ・Content-Type・上書き禁止条件を署名します。アップロード後にcomplete APIが実サイズを確認します。未完了オブジェクトはバケットのライフサイクルルールで削除してください。

Appは完了済みファイルを自分のメッセージへ添付できます。ダウンロード時に現在の閲覧権限を確認し、添付として配信します。

`link_domains` に登録したドメインのリンクを `link.shared` で受け取り、`unfurls` APIで共通カードを設定できます。Zooは任意のリンク先を取得しません。カードには提供Appを表示します。

## 既存Webhookと移行

既存のWebhookユーザーは、同じIDを持つ通知専用Appへ移行します。Webhookの受信仕様はSlack互換形式へ変更します。送信元のURLと本文形式の更新手順は [Webhook設定](webhooks.md) を参照してください。既存の投稿者・投稿ID・保存済み通知カードは保持します。削除済みのBotも過去ログの表示用に残します。同じ名前という理由では統合しません。

新しいWebhookも通知専用Appを使います。設定名とAppの表示名は独立しています。「投稿Appを変更」から、自分が管理する通知専用Appを複数のWebhookで共有できます。既存メッセージの投稿者IDは書き換えません。最後のWebhookを削除したAppは無効化し、過去のプロフィールは保持します。

通知専用Appの管理権限は従来のチャンネルWebhook権限を維持し、会話の読み取りやDMの権限を与えません。会話機能への拡張には管理者の承認が必要です。GitHub 専用の受信口と環境変数設定は削除しました。GitHub 連携は Bot アプリとして実装します。共通 Webhook の仕様は [Webhook設定](webhooks.md) を参照してください。

## 運用・検証

APIの失敗はHTTPステータスと `error` を返します。Appルートは `private, no-store` を返し、本番プロキシでも保持します。`/api/apps/metrics` は管理者に配信待ち・失敗・再送・最古の待ち時間を返します。ログはevent IDとdelivery IDで追跡でき、本文やSecretを記録しません。

検証コマンドは `make test-unit`、`make test-e2e`、`make frontend-check`、`make frontend-test`、`make sdk-check`、`make sdk-test` です。S3接続先の実運用検証は、対象バケットに対してアップロード・complete・ダウンロードを実施してください。

## Slack イベントの対応範囲

公式一覧との全件対応・例外理由は [Slack イベント対応表](slack-event-coverage.md)、機械可読の一覧は [slack-event-coverage.json](slack-event-coverage.json) です。`GET /api/apps/events` でも取得できます。Slack のイベント名も購読名として指定でき、対応する Zoo のイベント名で届きます。公開／非公開／DM／グループ DM の区別を保ちます。既存 Slack SDK のペイロードとは互換ではありません。

新しい scope は `users:write`、`preferences:read/write`、`pins:read/write`、`stars:read/write`、`emoji:read/write`、`usergroups:read/write`、`workspace:read/write`、`apps:read`、`invites:read/write`、`reminders:read/write`、`files:share`、`metadata:read/write`、`assistant:write`、`functions:execute` です。管理操作には scope に加えて操作主体の管理者権限が必要です。Bot に管理者権限を自動付与しません。

### 状態を変更する API

以下は人の認証を受け付けるパスです。App は対応する `/api/apps/v1/` パスと scope を使います。JSON フィールドの省略は現在値の維持です。時刻は RFC3339、明示的な `null` で期限を解除します。

| 操作 | API | 主な入力 |
| --- | --- | --- |
| 自分のプロフィール・状態・設定 | `GET/PATCH /api/users/me/profile` | `display_name`, `status_text`, `status_emoji`, `status_expires_at`, `fields`, `preferences`, `presence: auto/away`, `dnd_until` |
| 公開プロフィール | `GET /api/users/{id}/profile` | 私的な設定や DND 終了時刻は返さない |
| ゲスト・利用停止 | `PATCH /api/users/{id}/guest` | 管理者のみ。`kind: none/single/multi/deactivated` |
| 在席の更新 | `POST /api/presence/heartbeat` | 可視のクライアントが30秒ごとに更新、90秒で away |
| トピック・説明・投稿制限 | `GET/PATCH /api/channels/{channel}/settings` | `topic`, `purpose`, `posting_policy: members/managers` |
| 会話の表示・非表示 | `POST /api/channels/{channel}/view` | `open: boolean`。所属の追加・削除とは別 |
| 履歴の一括削除 | `DELETE /api/channels/{channel}/history` | チャンネル管理者のみ、メッセージを論理削除 |
| ピン | `GET/POST /api/channels/{channel}/pins` | `item_type: message/file`, `item_id` |
| ピンの解除 | `DELETE /api/channels/{channel}/pins/{kind}/{id}` | — |
| 自分の保存項目 | `GET/POST /api/stars` | `item_type: message/channel/file`, `item_id`, 必要なら `channel_id` |
| 保存の解除 | `DELETE /api/stars/{kind}/{id}` | — |
| カスタム絵文字 | `GET/POST /api/emojis`, `DELETE /api/emojis/{name}` | `name`, `url` (HTTPS)。リアクションは `:name:` |
| ワークスペース設定 | `GET/PATCH /api/workspace` | `name`, `preferences`, `profile_fields: [{id,label,type}]`。編集は管理者のみ |
| ユーザーグループ | `GET/POST /api/usergroups`, `PATCH /api/usergroups/{id}` | `name`, `handle`, `description`, `active`。編集は管理者のみ |
| グループメンバー | `GET/PUT /api/usergroups/{id}/members` | PUT はユーザー ID 配列、管理者のみ |
| 招待申請 | `GET/POST /api/invite-requests` | `email`, `channel_id` (任意) |
| 招待申請の判断 | `PATCH /api/invite-requests/{id}` | 管理者のみ、`status: approved/rejected` |
| リマインダー | `GET/POST /api/reminders`, `PATCH/DELETE /api/reminders/{id}` | `content`, `due_at`, `channel_id` (任意) |
| メッセージ metadata | `PATCH/DELETE /api/messages/{id}/metadata` | `event_type`, `event_payload`。投稿者のみ |
| Assistant セッション | `GET/POST /api/assistant-sessions`, `PATCH /api/assistant-sessions/{id}` | 作成: `app_id`, `user_id`, `channel_id`, `parent_id` (任意), `title`, `context`。更新: `title`, `context`, `state: stopped` |
| App の表示コンテキスト | `POST /api/apps/{id}/context` | `channel_id`, `context` |
| 外部アカウント接続要求 | `GET/POST /api/apps/{id}/connection` | `connect: boolean`, `channel_id` (任意)。manifest の `user_connections: true` が必要 |
| 接続結果の通知 | `POST /api/apps/v1/user-connections/{user}` | App が `connected: boolean` を返す。接続要求のある本人のみ |
| 関数の実行 | `POST /api/apps/{id}/functions/{name}/invoke` | manifest の `functions` に登録。`id` (任意、冪等キー), `channel_id` (任意), `inputs` |
| 関数の完了 | `POST /api/apps/v1/function-executions/{id}/complete` | 対象 App のみ、`outputs` または `error` |
| 外部エンティティの取得要求 | `POST /api/apps/{id}/entities/requests` | `kind: details/comments`, `url`, `message_id`, `channel_id`, `cursor`, `limit`。実際に unfurl されたリンクに限定 |
| ファイルの情報・編集・削除 | `GET/PATCH/DELETE /api/apps/v1/files/{id}` | `name`, `description`。削除後はダウンロード・公開リンクとも使用不可 |
| ファイルの共有先追加・削除 | `POST /api/apps/v1/files/{id}/shares`, `DELETE .../shares/{channel}` | `channel_id`。共有先へのアクセスが必要 |
| ファイルの公開リンク | `POST /api/apps/v1/files/{id}/public-link` | `public: boolean`。作成時だけ無作為な URL を返し、DB はハッシュだけ保存 |
| App の削除 | `POST /api/apps/{id}/actions/delete` | 開発者・管理者のみ。失効・チャンネル退出と論理削除。過去の投稿は保持 |

招待申請の承認は Zoo の管理記録です。外部 IdP でのアカウント発行は IdP 側で行います。カスタム絵文字の配信元は既存の App 画像と同様に CSP 設定が必要です。DND はプッシュ送信を抑止し、通知一覧には記録を残します。期限切れ状態とリマインダーはバックエンドのワーカーで処理します。ゲストは未参加の公開チャンネルも閲覧できず、single ゲストは通常チャンネル1件に制限されます。利用停止はキャッシュ済みの認証情報でも拒否します。

Assistant・外部接続・リンク詳細要求は対応する App にだけ配信します。外部サービス自体の接続処理や AI の生成処理は Bot 側の責務です。App 詳細画面から接続要求と Assistant の開始・改名・停止、展開済みリンクから詳細／コメント要求を実行できます。その他の追加リソースは上記 API で操作できます。

### 通信制御

WebSocket は接続時に `{"type":"hello","version":1,"installation_id":"..."}`、終了時に送信可能なら `{"type":"goodbye","reason":"connection_closed"}` を返します。ネットワーク切断では goodbye の到達は保証されません。通常の Delivery と区別し、ACK は Delivery に対してだけ返します。

`users:read` を持つ Socket から `{"type":"presence_query","id":"request-id","users":["user-id"]}` を送ると `{"type":"presence_snapshot","request_id":"request-id","users":[{"user_id":"...","presence":"active"}]}` が返ります。`presence_sub` は最大500件の購読集合を置き換え、変更時に snapshot を送信します。空配列で解除します。接続し直した際は再購読してください。権限のない要求は `type: error` になります。

HTTP App は `POST /api/apps/{id}/actions/verify-endpoint` で署名付き `{"type":"url_verification","challenge":"..."}` を受けます。同じ challenge を JSON オブジェクトまたはプレーンテキストで返すと検証時刻を記録します。応答上限は4KiB、リダイレクトは禁止、送信先は公開 IP に固定して検証します。manifest または署名鍵の変更で検証記録を解除します。検証の実行は任意です。サンプル HTTP Bot は challenge にも対応します。

通常イベントの配信はインストールごと毎時30,000件までとし、再送は同じイベントとして扱います。`app.uninstalled`・`tokens.revoked`・`app.deleted` と上限通知は、記録・初回配信の両方でこの上限の対象外です。超過すると `app.rate_limited` を1時間に1件配信します。発火側でも不要なキューの増大を抑止し、並行トランザクションの境界で作られた余剰分は配信側で上限を適用します。イベント ID はトランザクションと内容から決定し、同一操作による重複通知を避けます。

契約更新時は `python3 scripts/sync-app-contracts.py` で対応表・JSON Schema・OpenAPI・SDK のイベント型を再生成します。

## 認可・配信の境界

チャンネル本文・ファイル・リンク通知は、ユーザー代理トークンの同意だけでは配信しません。App の Bot 自体が参加している会話だけを対象にし、配信時にも Bot の参加状態を再確認します。公開チャンネルのメタデータ発見（`channels:read`）は例外ですが、本文と DM の内容は含みません。本人向けの既読・設定などは、Bot の参加が必要な場合に加えて、その本人の有効な同意が必要です。

`link.shared` は投稿 ID と登録ドメインに一致する `links` 配列だけを渡します。本文は `messages:read`、metadata は `metadata:read` が必要です。metadata のない資格情報には、通常の投稿イベント・変更前の値・返信・Shape のいずれからも metadata を返しません。

ユーザーの委譲トークンは、その人の招待申請と、そのトークンを発行した App の Assistant セッションだけを扱います。外部アカウント接続の確認 API は Bot トークンで呼び出してください。再承認・無効化では、古い private UI・応答トークンも失効します。対話イベントは `events` に購読を登録する必要があり、画面操作には現在の `ui:write` も必要です。

同意取消・認可コード交換・更新・再承認はインストール単位で直列化します。資格情報の scope が現在の承認範囲を超える場合、その資格情報は受け付けません。

Socket はインストールごと同時4接続まで、各接続の受信は毎秒20フレームまでです。`presence_query` / `presence_sub` は全接続合計で毎分30要求まで、最大500ユーザーです。在席の自動更新は5秒間隔、アイドル時の配信確認は1秒間隔です。切断後は接続枠を解放し、サーバー停止時の未解放枠も30秒で失効します。

HTTP は1インストールにつき最大32件を並行処理し、ワーカー全体では最大8インストールを処理します。通常配信と独立したメンテナンス処理で、30日を超える履歴を1トランザクション1,000件、1回最大10バッチずつ削除します。無効化後も購読済みの失効通知を配信し、Socket では `app.uninstalled` を優先してから残る失効通知を送ります。

GitHub 専用 Webhook を使っている場合の導入前手順は [リリースノート](releases/app-platform.md) を確認してください。

HTTP の候補は、未 ACK・送信可能時刻・リース・現在の閲覧権限を確認して選びます。候補として選んだ時刻をインストールごとに記録し、未選択・長く待った App から100件ずつ巡回します。複数ワーカーではロック済みのインストールを飛ばし、退出済み Bot の配信不能イベントや応答待ちのリースが候補枠を占めないようにします。

## 基盤を使ったアプリ

[GitHub 通知 Bot](../apps/github-bot/README.md) は、この基盤の HTTP 配信・投稿 API・汎用リンクカード・SDK を使う独立したアプリです。GitHub の変更を Zoo の返信・メッセージ編集へ反映します。Zoo 本体の GitHub 専用 Webhook は復活させません。
