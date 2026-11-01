# Home・チャンネル連携・ActivityPub

左レールの **Home** (`/home`) のTL右下にある投稿ボタンからフォームを開きます。PCではダイアログ、768px未満では全画面で表示します。フォーム右上は左から「公開範囲・連合あり／なし・投稿」のアイコンボタンです。閉じても下書きを保持し、送信成功時にクリアしてフォームを閉じます。投稿時に公開範囲と、必要であれば同期先の公開チャンネルを選択します。Homeでの新規投稿は最大1000文字です。チャンネルからLTLに流す本文・既存投稿の編集はチャンネルと同じ40000文字まで扱います。同じ投稿をHomeとチャンネルに表示し、どちらで編集・削除しても双方へ反映します。チャンネルではメッセージ時刻の右にActivityPubアイコンと公開範囲を表示します。HomeではActivityPubアイコンを表示せず、Zooのチャンネル由来の投稿は時刻の直後に「#チャンネル名」を表示します。公開範囲の選択肢は地球儀のパブリック・家のホーム（未収載）・鍵のフォロワー限定です。公開範囲と連合は独立して選択します。連合未設定時は連合なしで投稿でき、ホーム（未収載）の範囲はそのまま保ちます。

## 公開範囲

以下の外部配送・Note取得は「連合あり」の場合です。「連合なし」はどの公開範囲でも外部配送・未認証のNote取得を行いません。

| 設定 | Zoo内の表示 | 外部への配送 | 未認証のNote取得 |
| --- | --- | --- | --- |
| ローカルのみ | Home・LTL・指定チャンネル | なし (`apub=false`) | 不可 |
| パブリック | Home・LTL・指定チャンネル | 受理済みの外部フォロワー | 可 |
| ホーム（未収載） | 投稿者のHome・指定チャンネル | 受理済みの外部フォロワー | URLを知っていれば可 |
| フォロワー限定 | 投稿者のHome | 受理済みの外部フォロワーのみ | 不可 |

未収載・フォロワー限定投稿は公開Outboxの一覧に含めません。フォロワー限定は配送されたActivityの中で本文を渡します。署名付きGETによるフォロワー限定Noteの再取得は未対応です。チャンネルには参加者がフォロワー以外にもいるため、フォロワー限定投稿を紐づけることはできません。連合なしのフォロワー限定投稿は、ローカルユーザー間のFollowが未実装のため、投稿者のHomeにのみ表示します。同期先は参加済みで投稿権限のある公開・非アーカイブのチャンネルのみです。

Homeにはローカル投稿と、自分がフォローし、自分のInboxに届いた外部投稿を表示します。LTLにはローカルの「公開」「ローカルのみ」を表示します。ゲストには自身の投稿と参加先チャンネルの投稿のみ表示します。タイムラインは新しい順に50件ずつ表示します。WebSocketで常時接続し、投稿・編集・削除や連合受信時に現在の公開範囲を確認して更新します。読み進めている位置と読み込み済みの件数を保ち、切断時は自動で再接続します。スマホではTLの先頭で下へ引っ張り、離すと更新できます。

## チャンネル名ルール

Homeの「ActivityPub・チャンネル連携」で、ワークスペース管理者がLTLへ流すチャンネル名のルールを設定できます。SQL LIKE形式で、`%` は任意の文字列、`_` は任意の1文字です。例：`public-%`。バックスラッシュは通常の文字として扱います。

空欄で無効です。保存後に投稿された、条件に一致する公開チャンネルのメッセージをLTLに追加します。既存メッセージの遡及取り込み、非公開チャンネル、DMは対象外です。自動取り込みの投稿は `apub=false` であり、外部には配送しません。

チャンネルを非公開にするとローカル投稿はLTLから見えなくなります。連合済みの投稿が残るチャンネルを非公開にする操作は拒否します。先に該当投稿を削除してください。チャンネルの削除は紐づく投稿の削除・Delete配送を生成します。外部が保存したコピーの消去は保証できません。

## 連合の設定

1. DB migrations `20261101010000_create_home_posts.sql`、`20261101020000_activitypub.sql`、`20261101030000_decouple_home_federation.sql` を適用します。
2. バックエンドの `ACTIVITYPUB_BASE_URL=https://zoo.example.com` に外部公開する正規オリジンを設定します。パス・クエリは不可です。設定が不正な場合は起動時にエラーになります。
3. 公開HTTPSから `/api/activitypub/` と `/.well-known/webfinger` をバックエンドへ到達させます。リポジトリのTraefik設定には反映済みです。
4. バックエンドから、外部サーバーの公開IP・HTTPS/443への通信とDNS解決を許可します。リダイレクト・プライベートIPへの接続は拒否し、DNS結果を接続先へ固定します。

未設定でもHomeの連合なしの投稿と、チャンネル投稿のLTL掲載が使えます。Composeにも環境変数を渡す設定があります。HTTPは開発時のloopbackオリジンに限って設定できますが、外部配送先は常に公開HTTPS/443のみです。

正規URLはActor・投稿IDになるため、運用開始後に変更しないでください。ユーザーごとのRSA秘密鍵と配送キューはPostgreSQLに永続化します。DBのアクセス制限とバックアップには秘密鍵が含まれることを考慮してください。初期試作版の既存Home投稿は移行時にローカル投稿になります。自動で外部へ公開し直すことはありません。

## フォローと送受信

Home右上の人＋虫眼鏡アイコンからユーザー検索を開きます。Zoo内のユーザー名・表示名、または `@user@server`・HTTPSのActor URLで検索するとユーザー詳細を表示し、外部ユーザーは詳細からフォローできます。リモートからのAcceptを受け取るまで「承認待ち」と表示します。フォロー解除はUndoを配送します。外部からZooユーザーへのFollowは自動受理し、署名付きAcceptを配送します。外部からの検索には `@<Zooのユーザー名>@<Zooのホスト>` を使用します。

実装済みのActivityはFollow／Accept／Reject／Undo(Follow)／Create(Note)／Update(Note)／Delete(Note)です。署名はRSA-SHA256の従来形式のHTTP Signatureに対応します。RFC 9421のみを使う相手との互換性は対象外です。受信ではHTTP署名、SHA-256 Digest、日時、Actor・鍵の所有者、オブジェクトの投稿者とオリジンを検証します。受理記録と本文の更新は同一トランザクションで確定し、重複受信を抑止します。削除後に遅延したCreateが届いても復活させません。受信本文はHTMLからテキストに変換し、Reactのテキストとして描画します。

配送はDBの永続キューから行い、ユーザー・配送先ごとの順序を保ちます。期限付きリースで複数プロセスの取得を排他制御します。失敗時は指数バックオフで最大10回試行し、404/410・禁止先は打ち切ります。Homeの連携欄に配送待ち・失敗件数を表示します。鍵はHTTP Signatureの正規化ライブラリ `http-signature-normalization` と既存のOpenSSLで扱います。

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

ローカルの署名付きActorを用いたプロトコル試験で、Follow/Accept、送受信キュー、署名不正、受信Create/Update/Delete、リプレイ拒否、閲覧者ごとの隔離、チャンネルとの編集・削除同期を検証します。

```sh
cd backend
export ACTIVITYPUB_BASE_URL=https://zoo.example.com
export DATABASE_URL=postgres://...
cargo test --test activitypub_test --test home_test -- --test-threads=1

# 同じ環境変数で、模擬配送先への署名付き送信・再送・順序維持も検証
cargo test --lib activitypub:: -- --include-ignored --test-threads=1
```

実サービスのMastodon・Misskeyとの相互接続試験は別途必要です。現時点ではDM、Announce、Like、添付ファイルの連合、外部返信のチャンネル取り込み、ローカルユーザー同士のFollow、共有Inbox、NodeInfo、Actor移行・鍵ローテーションの管理UIは対象外です。外部Actorの鍵は5分キャッシュします。鍵変更時は再取得まで検証が失敗する場合があります。

画面のサンプルは [PC・スマホの画像](images/activitypub/README.md) を参照してください。

### TLの更新接続

`/api/home/live` はWebSocketです。接続後10秒以内の最初のフレームで、RESTと同じ `authorization`（または開発時のみ `x-dev-user`）をJSONで送信します。認証情報はURLに含めません。通知は本文や宛先を含まない `refresh` のみで、クライアントは認証付きRESTで再取得します。接続中は20秒間隔のPing、5分ごとの再認証を行います。

バックエンドごとに1本のPostgreSQL LISTEN接続を共有します。DBのコミットに合わせたNOTIFYにより、別プロセスでの投稿・チャンネル同期・Inbox受信も配信します。DB接続が復旧した場合も再取得を通知します。リバースプロキシは `/api/home/live` のWebSocket Upgradeを通してください。
