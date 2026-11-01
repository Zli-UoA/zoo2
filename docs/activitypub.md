# Home・チャンネル連携・ActivityPub

左レールの **Home** (`/home`) のTL右下にある投稿ボタンからフォームを開きます。PCではダイアログ、768px未満では全画面で表示します。フォーム右上は左から「公開範囲・連合あり／なし・投稿」のアイコンボタンです。閉じても下書きを保持し、送信成功時にクリアしてフォームを閉じます。投稿時に公開範囲と、必要であれば同期先の公開チャンネルを選択します。Homeでの新規投稿は最大1000文字です。チャンネルからLTLに流す本文・既存投稿の編集はチャンネルと同じ40000文字まで扱います。同じ投稿をHomeとチャンネルに表示し、どちらで編集・削除しても双方へ反映します。チャンネルではメッセージ時刻の右にActivityPubアイコンと公開範囲を表示します。HomeではActivityPubアイコンを表示せず、Zooのチャンネル由来の投稿は時刻の直後に「#チャンネル名」を表示します。公開範囲の選択肢は地球儀のパブリック・家のホーム（未収載）・鍵のフォロワー限定です。公開範囲と連合は独立した選択メニューで指定します。連合未設定時は連合なしで投稿でき、ホーム（未収載）の範囲はそのまま保ちます。

## 公開範囲

以下の外部配送・Note取得は「連合あり」の場合です。「連合なし」はどの公開範囲でも外部配送・未認証のNote取得を行いません。

| 設定 | Zoo内の表示 | 外部への配送 | 未認証のNote取得 |
| --- | --- | --- | --- |
| ローカルのみ | 投稿者のホーム・ローカル・ソーシャル・指定チャンネル | なし (`apub=false`) | 不可 |
| パブリック | 投稿者のホーム・ローカル・ソーシャル・指定チャンネル | 受理済みの外部フォロワー | 可 |
| ホーム（未収載） | 投稿者のホーム／ソーシャル・指定チャンネル | 受理済みの外部フォロワー | URLを知っていれば可 |
| フォロワー限定 | 投稿者のホーム／ソーシャル | 受理済みの外部フォロワーのみ | 不可 |

未収載・フォロワー限定投稿は公開Outboxの一覧に含めません。フォロワー限定は配送されたActivityの中で本文を渡します。署名付きGETによるフォロワー限定Noteの再取得は未対応です。チャンネルには参加者がフォロワー以外にもいるため、フォロワー限定投稿を紐づけることはできません。連合なしのフォロワー限定投稿は、ローカルユーザー間のFollowが未実装のため、投稿者のホーム／ソーシャルにのみ表示します。同期先は参加済みで投稿権限のある公開・非アーカイブのチャンネルのみです。

TLは「ホーム・ローカル・ソーシャル」を切り替えられます。ホームには自分の投稿と、フォローして自分のInboxに届いた外部投稿を表示します。ローカルにはZoo内の「公開」「ローカルのみ」、ソーシャルにはホームとローカルの両方を表示します。Zoo内のユーザー同士のフォローは未対応です。ゲストには自身の投稿と参加先チャンネルの投稿のみ表示します。タイムラインは新しい順に50件ずつ表示します。WebSocketで常時接続し、投稿・編集・削除や連合受信時に現在の公開範囲を確認して更新します。読み進めている位置と読み込み済みの件数を保ち、切断時は自動で再接続します。スマホではTLの先頭で下へ引っ張り、離すと更新できます。

## チャンネル名ルール

Zoo設定の「一般」で、ワークスペース管理者がLTLへ流すチャンネル名のルールを設定できます。SQL LIKE形式で、`%` は任意の文字列、`_` は任意の1文字です。例：`public-%`。バックスラッシュは通常の文字として扱います。

空欄で無効です。保存後に投稿された、条件に一致する公開チャンネルのメッセージをLTLに追加します。既存メッセージの遡及取り込み、非公開チャンネル、DMは対象外です。自動取り込みの投稿は `apub=false` であり、外部には配送しません。

チャンネルを非公開にするとローカル投稿はLTLから見えなくなります。連合済みの投稿が残るチャンネルを非公開にする操作は拒否します。先に該当投稿を削除してください。チャンネルの削除は紐づく投稿の削除・Delete配送を生成します。外部が保存したコピーの消去は保証できません。

## 連合の設定

1. DB migrations `20261101010000_create_home_posts.sql`、`20261101020000_activitypub.sql`、`20261101030000_decouple_home_federation.sql`、`20261101040000_home_live.sql`、`20261101050000_apub_interactions.sql`、`20261101060000_social_actions.sql`、`20261101070000_review_hardening.sql`、`20261101080000_verify_note_origin.sql` を適用します。
2. バックエンドの `ACTIVITYPUB_BASE_URL=https://zoo.example.com` に外部公開する正規オリジンを設定します。パス・クエリは不可です。設定が不正な場合は起動時にエラーになります。
3. 公開HTTPSから `/api/activitypub/` と `/.well-known/webfinger` をバックエンドへ到達させます。リポジトリのTraefik設定には反映済みです。
4. バックエンドから、外部サーバーの公開IP・HTTPS/443への通信とDNS解決を許可します。リダイレクト・プライベートIPへの接続は拒否し、DNS結果を接続先へ固定します。

未設定でもHomeの連合なしの投稿と、チャンネル投稿のLTL掲載が使えます。Composeにも環境変数を渡す設定があります。HTTPは開発時のloopbackオリジンに限って設定できますが、外部配送先は常に公開HTTPS/443のみです。

正規URLはActor・投稿IDになるため、運用開始後に変更しないでください。ユーザーごとのRSA秘密鍵と配送キューはPostgreSQLに永続化します。DBのアクセス制限とバックアップには秘密鍵が含まれることを考慮してください。初期試作版の既存Home投稿は移行時にローカル投稿になります。自動で外部へ公開し直すことはありません。

## フォローと送受信

Home右上の人＋虫眼鏡アイコンからユーザー検索を開きます。Zoo内のユーザー名・表示名、または `@user@server`・HTTPSのActor URLで検索するとユーザー詳細を表示し、外部ユーザーは詳細からフォローできます。リモートからのAcceptを受け取るまで「承認待ち」と表示します。フォロー解除はUndoを配送します。外部からZooユーザーへのFollowは承認待ちとして保存します。通知画面から本人が承認すると元のFollowをobjectに持つAccept、拒否するとRejectを配送します。Actorは `manuallyApprovesFollowers: true` を公開します。承認前の相手には公開・非公開いずれの投稿も配送しません。既存の承認済みフォロワーは移行時も保持します。外部からの検索には `@<Zooのユーザー名>@<Zooのホスト>` を使用します。

実装済みのActivityはFollow／Accept／Reject／Undo(Follow)／Create(Note)／Update(Note)／Delete(Note)です。署名はRSA-SHA256の従来形式のHTTP Signatureに対応します。RFC 9421のみを使う相手との互換性は対象外です。受信ではHTTP署名、SHA-256 Digest、日時、Actor・鍵の所有者、オブジェクトの投稿者とオリジンを検証します。受理記録と本文の更新は同一トランザクションで確定し、重複受信を抑止します。削除後に遅延したCreateが届いても復活させません。受信本文はHTMLからテキストに変換し、Reactのテキストとして描画します。

配送はDBの永続キューから行い、ユーザー・配送先ごとの順序を保ちます。期限付きリースで複数プロセスの取得を排他制御します。失敗時は指数バックオフで最大10回試行し、404/410・禁止先は打ち切ります。配送待ち・失敗件数は認証済みのステータスAPIで取得できます。鍵はHTTP Signatureの正規化ライブラリ `http-signature-normalization` と既存のOpenSSLで扱います。

## API

- `GET /api/home?timeline=home|local&before=<post-id>`：認証必須。`before` は省略可能。
- `POST /api/home`：`{"content":"近況","apub":true,"visibility":"public","channel_id":"任意のチャンネルID"}`。`apub` は公開範囲と独立した連合フラグです。`false` でも `public` / `unlisted` / `followers` を指定できます。両方省略した場合は従来互換の `local` 投稿です。
- `PATCH /api/home/<post-id>`：`{"content":"更新した本文"}`。公開範囲・紐づけ先は投稿後には変更できません。
- `DELETE /api/home/<post-id>`：投稿者本人のみ削除。
- `GET /api/home/settings`、`PUT /api/home/settings`：ルール取得・管理者による更新。
- `GET /api/activitypub/status`：自身のActor URLと配送件数。
- `GET/POST/DELETE /api/activitypub/follows`：自身のフォロー一覧・追加・解除。POST/DELETEは `{"actor":"..."}`。DELETEはActor URLを指定。
- `GET /.well-known/webfinger?resource=acct:<name>@<host>`：アカウント検索。
- `GET /api/activitypub/users/<user-id>`：Person、公開鍵、Inbox、Outbox。
- `GET /api/activitypub/users/<user-id>/outbox`：OrderedCollection。`first` / `next` で公開Create/Noteを50件ずつ取得。
- `GET /api/activitypub/users/<user-id>/followers|following`：人数のみ。個々のActorは非公開。
- `POST /api/activitypub/users/<user-id>/inbox`：署名必須、本文上限256 KiB。
- `GET /api/activitypub/notes/<post-id>`：公開・未収載Note。削除済みは410 Tombstone。

## 検証と対応範囲

ローカルの署名付きActorを用いたプロトコル試験で、Follow/Accept/Reject/Undo、Like/Undo、送受信キュー、署名不正、受信Create/Update/Delete、リプレイ時の重複処理防止、閲覧者ごとの隔離、チャンネルとの編集・削除同期を検証します。

```sh
cd backend
export ACTIVITYPUB_BASE_URL=https://zoo.example.com
export DATABASE_URL=postgres://...
cargo test --test activitypub_test --test home_test -- --test-threads=1

# 同じ環境変数で、模擬配送先への署名付き送信・再送・順序維持も検証
cargo test --lib activitypub:: -- --include-ignored --test-threads=1
```

実サービスのMastodon・Misskeyとの相互接続試験は別途必要です。現時点では独立したDM、引用リポスト、添付ファイルの連合、外部返信のチャンネル取り込み、ローカルユーザー同士のFollow、共有Inbox、NodeInfo、Actor移行・鍵ローテーションの管理UIは対象外です。外部Actorの鍵は5分キャッシュします。鍵変更時は再取得まで検証が失敗する場合があります。

画面のサンプルは [PC・スマホの画像](images/activitypub/README.md) を参照してください。

### TLの更新接続

`/api/home/live` はWebSocketです。接続後10秒以内の最初のフレームで、RESTと同じ `authorization`（または開発時のみ `x-dev-user`）をJSONで送信します。認証情報はURLに含めません。通知は本文や宛先を含まない `refresh` のみで、クライアントは認証付きRESTで再取得します。接続中は20秒間隔のPing、5分ごとの再認証を行います。

バックエンドごとに1本のPostgreSQL LISTEN接続を共有します。DBのコミットに合わせたNOTIFYにより、別プロセスでの投稿・チャンネル同期・Inbox受信も配信します。DB接続が復旧した場合も再取得を通知します。リバースプロキシは `/api/home/live` のWebSocket Upgradeを通してください。

投稿の下には返信・リポスト・リアクション・3点メニューを表示します。自分の投稿の編集・削除は、この3点メニューから操作します。投稿ボタンは投稿数に関わらずTL右下に配置します。スマホではフォームを開くタップ内で同期的に入力欄へフォーカスし、キーボードを呼び出します。実際のIMEの描画タイミングはOSとブラウザに依存します。


## リアクションと通知

Homeの投稿下のリアクションボタンから絵文字を選択できます。自分のリアクションを押すと取り消せます。外部投稿へのいいねはActivityStreamsの `Like`、取り消しは元のLikeをobjectに含む `Undo` を、投稿者のInboxだけに署名付きで配送します。外部から自分の投稿へのLike/Undoも受信し、TLの件数を更新します。公開・未収載Noteの `likes` は人数だけを返すCollectionです。フォロワー限定・連合なしの投稿のCollectionは公開しません。Zoo内だけのいいねは連合設定がなくても利用できます。

通知画面の「ホームの通知」に、フォロー申請、フォロー承認、いいねを表示します。ベルの未読件数には既存のチャンネル通知とホーム通知を合算します。通知はWebSocketで更新し、個別・一括で既読にできます。1ページ100件を表示し、「過去の通知を読む」から続きを取得できます。未処理のフォロー申請を優先し、100件を超えた申請も既読状態にかかわらず承認・拒否できます。一括既読は表示外の古い通知にも適用します。申請への承認・拒否は通知の所有者のみ操作でき、承認待ちの状態とActivity IDを確認して排他制御します。この通知はアプリ内通知で、ホーム通知のWeb Push配送は未対応です。

### ActivityPubの重点テスト

[ActivityPubのServer-to-Server Interactions](https://www.w3.org/TR/activitypub/#server-to-server-interactions) と [ActivityStreams 2.0 Vocabulary](https://www.w3.org/TR/activitystreams-vocabulary/) に基づき、対応しているActivityについて以下を検証します。完全なJSON-LD処理や公式の完全適合認証を行うものではありません。

- Actor・Note・CollectionのID、公開鍵のowner、RFC 3339の日時、HTMLのエスケープ。
- 3種類の公開範囲 × Create/Update/Deleteの送信スナップショット、to/cc、ActorとattributedTo、Tombstone、再送時のActivity ID維持。
- 承認前には配送しないこと、Accept/Rejectが元のFollowを参照すること、同時承認でAcceptを二重発行しないこと、古い申請や他人の申請を操作できないこと。
- Like/Undoの所有Actor・対象Note・宛先、文字列と埋め込みオブジェクトによるUndo、順序が逆転したUndo、重複受信、古いUndoと新しいLikeの分離。
- フォロワー限定Note・受信者限定の外部Noteの閲覧とリアクション、通知のユーザー間隔離・既読権限。
- OpenSSLで独立に組み立てた受信署名と、模擬HTTPサーバーへの署名付きAccept/Reject/Like/Undoの実バイト列・Content-Typeの照合。署名方式は既存のMastodon互換HTTP Signatureで、RFC 9421形式は未対応です。

`backend/tests/activitypub_test.rs` と `backend/src/activitypub/tests.rs` が主な検証箇所です。上記のコマンドでDBを必要とする試験を明示的に実行します。実際のMastodon・Misskeyとの相互接続試験は未実施です。

互換性の制限として、Actor・公開鍵・Inbox・followers・受信Activity/NoteのIDは同一オリジンの構成に限定します。これらが別オリジンとなるサーバーや、独自JSON-LD表現を含むすべての実装との互換性は保証していません。


## 返信・リポスト・Misskeyのカスタム絵文字

連合先のIDは `@ユーザー名@ホスト名` と表示し、ホスト部分を薄めの色にします。表示名と本文のEmojiタグ、およびリアクションのカスタム絵文字を描画します。画像取得に失敗した場合は `:名前:` に戻します。

- 返信ボタンは既存の投稿フォームを返信先付きで開きます。スマホではタップ内で入力欄へ同期フォーカスします。元投稿が連合なしなら返信も連合なし、フォロワー限定なら返信もフォロワー限定とし、未収載への返信を公開へ拡大することも拒否します。配送するNoteには `inReplyTo` と元の投稿者へのMention・ccを含め、フォローされていない相手にも返信・編集・削除を直接配送します。
- リポストは `Announce`、取り消しは元のAnnounceをobjectに持つ `Undo` です。自分のリポストはTLに元投稿と区別できる項目として並び、カーソルは項目ごとのIDを使います。フォロワー限定の投稿のリポストは拒否し、元投稿の削除や公開範囲の変更後はアクセスできない本文を表示しません。第三者のAnnounceの埋め込み内容は信用せず、未知の元投稿は公開HTTPSの正規URLから取得して、ID・attributedTo・公開範囲を確認します。
- リアクションの選択では共通のUnicode絵文字ピッカーと、Misskeyのカスタム絵文字一覧を使います。Misskeyと一部の互換実装はNodeInfoで識別し、公開 `GET /api/emojis` を取得します。カタログが取得できない場合も、その投稿のEmojiタグや受信済みリアクションを利用できます。カタログは最大8 MiB／10000件、選択画面は検索結果の先頭200件を表示します。
- Misskeyの形式に合わせ、`Like.content`・`Like._misskey_reaction` に絵文字を入れ、カスタム絵文字はEmojiタグを添付します。一人につき一つのリアクションを保持し、変更時は元のLikeのUndoと新しいLikeを順序付きで配送します。連合先の受付設定によっては、先方で通常のいいねへ変換される場合があります。表示件数はZooで把握している受信・操作分で、連合先サーバー全体の集計値ではありません。

Misskeyの形式は [ApRendererService](https://github.com/misskey-dev/misskey/blob/develop/packages/backend/src/core/activitypub/ApRendererService.ts)、[ApInboxService](https://github.com/misskey-dev/misskey/blob/develop/packages/backend/src/core/activitypub/ApInboxService.ts)、[emojis API](https://github.com/misskey-dev/misskey/blob/develop/packages/backend/src/server/api/endpoints/emojis.ts) の公開実装と照合しています。実際のMisskeyサーバーとの相互接続試験は未実施です。

絵文字画像は、受信タグ・カタログから登録した画像IDだけをZooのプロキシで公開します。任意URLの画像プロキシにはしません。HTTPS・公開IP・DNS固定・リダイレクト禁止を適用し、画像は最大2 MiB、同時取得8件に制限します。サーバー側では成功した画像を5分、取得失敗を30秒キャッシュし、最大128件・合計32 MiBまで保持します。キャッシュにない画像の取得は接続元IPごとに毎分240件、プロセス全体で毎分960件までです。Cloudflare Tunnel用の隔離されたオリジンでは `ACTIVITYPUB_CLOUDFLARE_TUNNEL=true` とし、プライベートIP・loopbackのプロキシ接続から届く `CF-Connecting-IP` を利用者ごとの制限に使います。`compose.stg.yaml` はこの構成を既定で有効にします。CloudflareのPseudo IPv4による上書き時には `CF-Connecting-IPv6` の実IPv6を使用します。公開IPからの直結、または設定が無効な場合は転送ヘッダーを信頼せずTCP接続元を使います。Tunnel設定でヘッダーや接続元が不明な場合は400とし、プロキシ全体の共有枠には戻しません。429には `Retry-After: 60` と `Cache-Control: no-store` を返します。画像登録は未使用30日で削除し、認証済みのTL・通知・カタログ閲覧時に使用中の登録を更新します。SVGを含む対応画像には `nosniff` とsandboxのCSPを付けます。

APIの追加・拡張：

- `POST /api/home` の `in_reply_to`：返信先のHome投稿IDまたは受信Note URI。
- `POST/DELETE /api/activitypub/reposts`：`{ "post_id": "..." }`。
- `POST /api/activitypub/reactions`：`post_id` に加え `reaction`、カスタムの場合は選択肢から得た `emoji_id` を指定。DELETEで自分のリアクションを取り消します。
- `GET /api/activitypub/reaction-options?post_id=...`：閲覧できる投稿のリアクション用絵文字一覧。
- `GET /api/activitypub/emoji-images/<登録済み画像ID>`：制限付きの画像取得。

追加テストではMisskey形式のタグ・Like/Undo、リアクション変更順序、画像の内部IPへの取得拒否、返信先の権限と公開範囲、直接配送先へのUpdate/Delete、Announce/UndoとTLページ送りを検証します。


## レビューで追加した保護

受信履歴はActivity ID・Actor・受信ユーザーの組で管理します。未知のNoteのDeleteはActorを含む別の削除記録に保存し、同じオリジンの別ActorによるIDの先取りで本来の投稿を抑止しないようにします。新しく取得したActor情報と絵文字URLは、Inbox署名の検証成功後に保存します。

Homeの追加読み込み中に届いたlive refreshは、読み込み完了後へ延期します。再取得ではDBが返す正確なepochマイクロ秒文字列と末尾IDを使い、既に読み込んだ末尾まで取得します。JavaScriptの日時への丸めやIDの照合順序には依存せず、新着の追加で古い投稿が押し出されないようにします。明示的な更新とTL切り替えでは進行中の取得を取り消します。

DB migrationは既存の運用と同じforward-onlyです。適用前にDBをバックアップしてください。アプリだけ切り戻す場合も追加テーブルをその場で削除せず、修正migrationでの復旧かバックアップからの復元を選びます。ActivityPubの秘密鍵を含むDBバックアップにはアクセス制限が必要です。


Note IDが別Actorのキャッシュ行と衝突した場合は、そのIDをHTTPSで再取得して原本の `attributedTo` を確認します。最初に届いたActorだけでは所有者を確定しません。誤った行を置き換える場合は、旧行への受信権限・リアクション・リポスト・通知も破棄します。原本確認済みのIDは別Actorへ引き渡しません。Announceの既知行による取得省略も、原本確認済みの場合だけに限定します。ネットワーク取得はDB接続を保持せず行い、競合が後から発生した場合は受信履歴を確定せず再送で原本確認を行います。
