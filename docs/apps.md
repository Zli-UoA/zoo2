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

App設定の `scopes` は `messages:read`、`messages:write`、`reactions:write`、`channels:read`、`channels:manage`、`users:read`、`dm:write`、`files:read`、`files:write`、`ui:write`、`commands:read`、`links:write` を受け付けます。

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

配信は最低1回で、厳密な順序は保証しません。イベントIDで重複を排除してください。SDKは同時処理と直近1,000件を重複排除します。再起動をまたぐ排除には `EventStore` を実装し、副作用も冪等にしてください。ACK期限は3秒、失敗時は指数バックオフ（最大1時間）で最大24時間再送します。配信履歴は30日保持し、開発者・管理者が確認・再送できます。

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

既存のWebhookユーザーは、同じIDを持つ通知専用Appへ移行します。URL・Secret・署名・重複排除・投稿ID・カード形式を変えません。削除済みのBotも過去ログの表示用に残します。同じ名前という理由では統合しません。

新しいWebhookも通知専用Appを使います。設定名とAppの表示名は独立しています。「投稿Appを変更」から、自分が管理する通知専用Appを複数のWebhookで共有できます。既存メッセージの投稿者IDは書き換えません。最後のWebhookを削除したAppは無効化し、過去のプロフィールは保持します。

通知専用Appの管理権限は従来のチャンネルWebhook権限を維持し、会話の読み取りやDMの権限を与えません。会話機能への拡張には管理者の承認が必要です。環境変数によるGitHub受信口も引き続き利用できます。

## 運用・検証

APIの失敗はHTTPステータスと `error` を返します。Appルートは `private, no-store` を返し、本番プロキシでも保持します。`/api/apps/metrics` は管理者に配信待ち・失敗・再送・最古の待ち時間を返します。ログはevent IDとdelivery IDで追跡でき、本文やSecretを記録しません。

検証コマンドは `make test-unit`、`make test-e2e`、`make frontend-check`、`make frontend-test`、`make sdk-check`、`make sdk-test` です。S3接続先の実運用検証は、対象バケットに対してアップロード・complete・ダウンロードを実施してください。
