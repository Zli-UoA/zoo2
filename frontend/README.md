# Frontend

Zoo2 のフロントエンド。TanStack Start（React 19）+ Vite+ + Tailwind CSS v4 + shadcn/ui（Base UI）。

この README は先に「現在の設計」で今の動きを説明し、実装の経緯は末尾の「履歴（実装の経緯）」にまとめています。履歴の段階名は当時の作業区分であり、現在の設定・手順は「現在の設計」を参照してください。

## 現在の設計

表示データは Electric Shape で読み取り、変更は REST で書き込み、書き込み後の楽観状態は `txid` で確定させます。認証は開発時の `X-Dev-User` と OIDC PKCE の2経路で、画面データはユーザー単位のオフライン DB に永続化します。文言は i18n に集約し、テストはユニット・ブラウザ・E2E・Storybook の入口に分かれています。

### データフロー

- 読み取り: `@tanstack/electric-db-collection` で Electric Shape を購読し、`@tanstack/react-db` の `useLiveQuery` で表示します。`src/lib/electric/collections.ts` が `channels` / `users` / `channel_user` / `messages` のコレクションを持ち、`messages` は `channel_id` 単位で生成してキャッシュします。ShapeStream はブラウザ API に依存するため、モジュール直下では作らず client コンポーネント / フックから `getCollections()` を呼びます（SSR では購読しません）。`src/hooks/useRealtime.ts` がチャンネル一覧・メッセージ・メンバー・ユーザーを購読します。
- Shape クエリ: `src/lib/electric/query.ts` が `buildShapeUrl()` に渡すクエリを組み立てます。`direct` では Electric の subset プロトコル（`where` + `params[1]`）、`proxy` では `channel_id` を渡します（`table` は direct では `buildShapeUrl()`、proxy ではバックエンドが付与）。`messages` は `channel_id` 付きのコレクションをチャンネルごとに購読しますが、`channelUsers` は `channel_id` なしの1本の Shape だけを購読し、`useChannelMembers()` が行を `channel_id` で絞り込みます（`query.ts` の `channelUsers` + `channelId` 分岐は将来チャンネル単位で購読するとき用で、現状は未使用）。
- 行の正規化: integer は文字列で届く前提で `coerceElectricRow()` が `Number()` 変換します（`int8` / `numeric` は精度のため文字列のまま）。timestamp は Shape が `2026-10-15 14:32:44.183`（空白区切り）、REST が `T` 区切りの文字列で、どちらも UTC の naive 値です。`normalizeUtcTimestamp()` / `parseUtcTimestamp()` で UTC の ISO 文字列・`Date` に正規化できます。ShapeStream の `transformer` でも integer を number、boolean の `"true"` / `"false"` を真偽値へ変換し、`"false"` が truthy になるのを防ぎます。
- 書き込み: すべて REST で行い、`src/lib/electric/writes.ts` のコレクションごとの `onInsert` / `onUpdate` / `onDelete` から呼びます。失敗時は throw して TanStack DB に楽観状態をロールバックさせます。
  - messages: 送信 `POST /api/messages`、編集 `PATCH /api/messages/{id}`、論理削除 `DELETE /api/messages/{id}`。
  - channels: 作成 `POST /api/channels`、更新 `PATCH /api/channels/{id}`。
  - channel_user: 参加 `POST /api/channels/{id}/join`、メンバー追加 `POST /api/channels/{id}/users`、本人の退出 `POST /api/channels/{id}/leave`。`/leave` は認証ユーザー自身だけを外す API なので、コレクションの `delete` では `getCurrentUserId()`（dev / OIDC のどちらでも解決した現在ユーザー。`config.devUserId` の固定値ではない）と行の `user_id` を照合し、自分以外なら throw して楽観状態を戻します。他人を外す管理操作は別に `DELETE /api/channels/{id}/users/{user_id}`（`useChannelAdmin().kickMember()`）を使います。
  - 論理削除は Electric 上では `update`（`deleted_at` の更新）として流れます。そのため楽観更新も `update(deleted_at)` に揃え、`delete→insert` の再出現（ちらつき）を避けています。`messageFeedQuery` が `deleted_at` を除外します。`messages` コレクションは `syncMode: "on-demand"` で全行を同期せず、`useMessageFeed()` が新しい方から `MESSAGE_PAGE_SIZE`（50）件ずつ取り、`loadOlder()` で古い方へ範囲を広げます。未取得の古い履歴は表示に含まれない前提で読んでください。投稿者名などは `messages` 行の `display_data.author` に保存されます（`infra/db/schema.sql` の `refresh_message_display`）。`messageFeedQuery` は `display_data.author.id` が行の `user_id` と一致すれば保存済みの名前・アイコンなどを優先し、一致しない旧キャッシュや楽観更新の行だけ `users` Shape で補います（`src/lib/electric/messageFeed.ts`）。
- 確定（`txid`）: `src/lib/electric/txid.ts` の `syncTxid()` と `changeMessageMatcher()` が REST の `{ data, txid }` から数値化した `txid` を `collection.utils.awaitTxId()` に渡します（EDC の `Txid` は number）。`VITE_TXID_MODE` で `awaitTxId`（既定）/ `awaitMatch`（operation と行 id で照合）/ `skip`（待機しない）を切り替えられます。`awaitTxId` は対象コレクションが live query で購読されている必要があります（画面に出しておく）。未購読だとタイムアウトします。
  - `awaitTxId` / `awaitMatch` がタイムアウトしても楽観状態はロールバックされ、UI には操作失敗と出ますが、サーバ側の書き込みは成功しているため少し遅れて Electric から同じ行が届き表示は戻ります。「同期待ち」と「失敗」を分けた表示は今後の課題です。
  - channel_user の書き込みは channel_user コレクションで `txid` を待ちます。一般ユーザーの proxy の channels Shape の行が消えるのは `is_private = false OR id IN (...)` の閲覧条件を満たさなくなったときだけです（例: 非公開チャンネルからの退出）。一般ユーザーは公開チャンネルから退出しても `is_private = false` が成立するので行は残り、サイドバーから消えるのは `ChannelSidebar` がメンバーシップで絞るためです。ゲストは `channels_shape` が参加中のチャンネルだけに絞るため、公開チャンネルから退出すると Shape の行も一覧からも消えます。条件の出入りによる move-in / move-out 自体は同じトランザクションの `txid` で配信されますが、書き込みハンドラが待つのは channel_user コレクションの `txid` です（チャンネル一覧は続けて更新されます）。
- バックエンドとの境界:
  - 読み取りは `proxy` では `VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`（既定 `/api/shapes`）を購読します。バックエンドが `table` / `where` を確定し、サーバ側で `ELECTRIC_SECRET` を付けて Electric へ転送します。
  - 認可: 一般ユーザーの channels Shape は `is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = ...)` で絞り、公開チャンネルは未参加でも一覧に出ます。未参加の非公開チャンネルは Shape・メッセージのどちらも届きません（存在隠蔽のため 404）。Shape の行が move-in / move-out するのはこの where 条件の結果が変わったときだけです。公開チャンネルは退出しても条件を満たすため行が残り、一覧から消えるのは `ChannelSidebar` がメンバーシップで絞るためです（`src/components/chat/ChannelSidebar.tsx`）。ゲストは `channels_shape` で参加済みのチャンネルだけに絞られ、未参加の公開チャンネルは一覧に出ず、メッセージ Shape の要求も 404 になります（`backend/src/routes/shapes.rs`）。
  - `channel_users` のサブクエリ経路（`electric#3143`）: Electric 1.8.1 を `ELECTRIC_FEATURE_FLAGS=allow_subqueries` で動かしており、`WHERE ... IN (SELECT ... WHERE user_id = $1)` の形がそのまま通ることを実測しました（`params[1]` により move-in / move-out も配信）。`$1` をリテラルへ埋め込む回避策は不要でした。
  - ShapeStream 用 fetch（`src/lib/electric/headers.ts`）は REST と同じ `Bearer` / `X-Dev-User` を「値があるときだけ」付与します。常時付与すると開発時の Electric 直結で `authorization` が CORS の preflight に弾かれるためです。`direct` は Electric の CORS が `X-Dev-User` / `Authorization` を許可しないため認証ヘッダを付けません（`collections.ts` の `shapeAuthSource`）。
  - CORS はバックエンドが `CORS_ALLOWED_ORIGINS` の許可オリジンに限定し（`ENVIRONMENT=production` 以外で未設定なら `http://localhost:5173`, `http://127.0.0.1:5173`、本番で未設定なら空）、メソッドとヘッダも既知のもの（ヘッダは `Authorization`、`X-Dev-User`、`Content-Type`、`Accept`、E2EE の直前の再認証の `X-Zoo-Reauth`、鍵バックアップの証明の `X-Zoo-Backup-Proof`、トレース伝播の `traceparent` / `tracestate`）だけを許可しつつ `electric-*` を公開します（実際の許可一覧は `backend/src/routes/mod.rs`）。
- 画面: ルート `/` は `src/routes/_shell.index.tsx` → `src/components/chat/WelcomePanel.tsx`（チャンネル未選択のメイン面）で、チャンネル一覧・作成・参加/退出は `src/components/chat/ChannelSidebar.tsx` と `ChannelMembersDialog.tsx` にあります。ルート `/channels/$channelId` は `src/routes/_shell.channels.$channelId.tsx` → `src/components/chat/ChannelScreen.tsx` → `src/components/chat/messages/MessagePane.tsx` で送信・編集・論理削除・投稿者表示を組みます。`src/hooks/useWriteRunner.ts` が `collection.insert()` などの楽観更新トランザクションを `when("settled")` まで待ち、失敗を UI のエラーメッセージにまとめます。`/realtime` は検証 UI から上記の実ルートへ移行済みで、現在は接続モード（Shape / txid / 開発ユーザー）を表示する最小画面（`src/components/SyncStatus.tsx`）です。
  - 編集は自分のメッセージにのみ表示します。削除は通常チャンネルでは投稿者本人に加えてチャンネル作成者とワークスペース管理者にも表示し、DM は投稿者本人だけです（`canModerate` は `!channel.is_dm && canManageChannel`。バックエンドの `can_delete_message` と一致します）。
  - 非公開チャンネルは参加者だけに説明・メンバー・メッセージを表示します。`proxy` の Shape はバックエンドが認可で絞るため未参加者には届きませんが、UI 側でも `isMember` で隠します。`direct` は認可せず全件配信するため UI 側のガードだけが頼りです。`/channels/$channelId` を URL 直開きしても未参加ならメッセージは出しません。説明はチャンネルのヘッダーに表示し、メンバーのダイアログには出しません。
  - メンバー追加フォームはバックエンドの `add_channel_user` と同じ役割別条件（管理者または作成者、非公開チャンネルは作成者または管理者のみ、公開チャンネルは作成者・管理者またはメンバー）で表示し、アーカイブ済みでは隠します。表示にはさらに参加要件が加わり、`ChannelMembersDialog` の `canViewMembers` は「公開チャンネルまたは参加済み」なので、非公開チャンネルでは未参加の管理者・作成者にメンバー欄（フォームを含む）が表示されません。バックエンドは暗号化されていない非公開チャンネルでは未参加の管理者・作成者でも追加を許可するため、API の権限と UI の表示条件は一致しません。暗号化チャンネルでは `channel_user` に実際に参加している人だけが追加でき、未参加の管理者・作成者でも `E2EE_ADD_FORBIDDEN`（403）で拒否されます。DM とアーカイブ済みチャンネルへの追加もバックエンドが拒否します。他人を外す kick は `DELETE /api/channels/{id}/users/{user_id}` で、通常チャンネルの作成者またはワークスペース管理者だけに許可され（DM は不可、作成者と管理者は除外できない）、本人の `/leave` とは別の操作です。重複メンバーの追加は UI 側で弾き、コレクションの `delete` で自分以外の行を `/leave` に送ると書き込みハンドラが throw してロールバックします。メンバー読み込み中は重複判定が効かないため参加/追加ボタンを無効化します。
  - メンバー操作は `ChannelMembersDialog`（`src/components/chat/ChannelMembersDialog.tsx`）にまとめています。`channel_user` の Shape は `getCollections()` が `channel_id` なしの1本だけ作り、ダイアログはそのコレクションを購読します。`users` は同一クエリのため TanStack DB 内部でプール共有されます。同じ channel の `useChannelMembers`（画面とダイアログ）もプールされ共有されるため、購読はチャンネル数に対して増えません。

### 設定と接続先

- `src/lib/config.ts` の `resolveConfig()` / `buildShapeUrl()` が設定を解決します。`VITE_SHAPE_MODE` が `direct` なら Electric（`VITE_ELECTRIC_SHAPE_URL`）、`proxy` ならバックエンドの Shape プロキシ（`VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`）を参照します。**既定は `proxy`**（開発・本番とも）。不正な値は例外にします。`direct` は Electric を認可なしで直接叩く開発専用で、プライベートチャンネルのフィルタも効きません。
- `proxy` モードの開発では `VITE_DEV_USER_ID`（`X-Dev-User` として送信。DB の `users` に存在する id）が必須で、未設定なら起動時エラーになります。**OIDC 設定時（`VITE_OIDC_ISSUER` と `VITE_OIDC_CLIENT_ID` の両方を指定）は `Bearer` で認証するため不要**です。`direct` でも不要です。バックエンドの既定ユーザーは `DEV_DEFAULT_USER_ID`（既定 `00000000-0000-0000-0000-000000000001`）で、`X-Dev-User` が無いときはこのユーザーとして扱われます。
- `src/lib/auth.ts` は `getToken()` のみのアダプタで、`setTokenProvider()` で IdP を差し替えられます（既定は未認証で `null`）。`buildAuthHeaders()`（`api/client.ts`）を使うと REST 側の fetch に同じ `Bearer` / `X-Dev-User` を渡せます。`src/lib/api/client.ts` が REST の `{ data, txid }` エンベロープを取り出します（`unwrapEnvelope()`）。
- 環境変数の一覧と既定値は [frontend/.env.example](.env.example) を参照してください。同ファイルは dev ユーザー経路を前提に書かれているため、OIDC 設定時の例外は上の説明を優先してください。

### 認証

開発時は `X-Dev-User`、OIDC を有効にしたときは Authorization Code + PKCE (S256) で認証します。`VITE_OIDC_ISSUER` と `VITE_OIDC_CLIENT_ID` を設定すると OIDC が有効になり、未設定なら従来どおり `X-Dev-User` の dev 経路で動きます。REST と Shape の両方で同じヘッダを受け付け、バックエンドは `DEV_AUTH_ENABLED=true` のときだけ `X-Dev-User` を受け付けます。`DEV_AUTH_ENABLED=false` で `Authorization` が無い場合は 401（`Authorization header is required`）で、`X-Dev-User` だけでは dev 経路にフォールバックしません（`backend/src/auth.rs`）。

AuthZ の public client には redirect URI（既定 `${origin}/auth/callback`）を登録し、`frontend/.env` に次を設定します。

```bash
# 本番: https://account.zli.works/api/auth / ローカル: http://localhost:4321/api/auth
VITE_OIDC_ISSUER=http://localhost:4321/api/auth
VITE_OIDC_CLIENT_ID=<AuthZ で登録した public client ID>
VITE_OIDC_REDIRECT_URI=http://localhost:5173/auth/callback
VITE_OIDC_SCOPE=openid profile
VITE_OIDC_RESOURCE=https://zli.chat
```

- 既定 scope は `openid profile` です。`VITE_OIDC_SCOPE` で上書きできます。追加属性と自動更新が必要な場合は、AuthZのclientで許可したscopeを追加してください。`ains` も現時点では要求せず、必要になったら `VITE_OIDC_SCOPE="openid profile ains"` のように追加します。
- `VITE_OIDC_RESOURCE` は AuthZ の resource（RFC 8707）です。設定すると authorize と token 要求の両方に `resource` が付き、access token が `typ: at+jwt` の JWT になります。未設定だと AuthZ は opaque token（32 文字）を発行します。本番の resource identifier は `https://zli.chat`、ローカルは AuthZ 側で登録した値を使います。
- AuthZ 側では resource identifier を `oauthResource` に登録し、zoo2 の client を `oauthClientResource` で紐付けます。AuthZ の管理画面で設定し、AuthZ の `docs/oidc.md` / `docs/deployment.md` も参照してください。
- バックエンドの `OIDC_AUDIENCE` には `VITE_OIDC_RESOURCE` と同じ resource identifier を設定します（access token の `aud` は resource の配列）。`resource` を設定しないと opaque token のためバックエンドの JWT 検証は通りません。`OIDC_ISSUER` も `VITE_OIDC_ISSUER` と一致させます。
- AuthZは `offline_access` に対応しています。管理者がclientに同scopeを許可し、`VITE_OIDC_SCOPE="openid profile offline_access"` で要求して本人が同意すると、refresh tokenでaccess tokenを更新します。refresh tokenがない場合は期限前に再認可します。同じタブのリロードでは保存済みセッションを復元します。期限が近づいて再認可が必要になるまでは、authorize への往復を省きます。
- ログアウトは Discovery の `end_session_endpoint` を使う RP-initiated logout です。`VITE_OIDC_POST_LOGOUT_REDIRECT_URI` で戻り先を変更できます（既定 `${origin}/`）。

#### 自動更新の有効化

1. AuthZの管理画面で、zoo2に登録したpublic clientに`offline_access`を許可します。redirect URI・resourceの既存設定は維持します。
2. ローカルは`frontend/.env`、本番は`Zli-UoA/zoo2`のRepository Variable `VITE_OIDC_SCOPE`に、現在のscopeへ`offline_access`を追加した値を設定します。通常の例は`openid profile offline_access`です。`ains`などを利用中なら、そのscopeも残します。
3. Webを再ビルドして配備します。`VITE_OIDC_*`はビルド時に埋め込むため、コンテナの再起動だけでは反映されません。本番の`Publish images`は同じSHAのイメージがあるとビルドを省略するため、変数の設定後に新しいコミットのSHAでビルドし、そのイメージをGitの配備設定へ反映します。詳細は[zoo2-infraの登録・配備手順](https://github.com/Zli-UoA/zoo2-infra/blob/main/docs/installation.md#authzへのzoo2の登録)を参照してください。
4. 利用者が再ログインし、`offline_access`の追加同意を行います。更新トークンの発行と、再読み込み後も自動更新が続くことを確認します。

AuthZ側の許可を先に追加し、続いてWebの設定・ビルド・配備、最後に本人の同意という順で進めます。設定例の変更だけで本番のclientや配備済みWebが変わることはありません。

### オフライン永続化

画面データは Electric コレクションをユーザー単位の OPFS SQLite DB に永続化します。オフライン再読み込みでは、シェルは Service Worker がキャッシュした `/index.html` または `/` を使い、どちらも見つからない場合だけ `/offline.html` にフォールバックします。本文のメッセージは `useMessageFeed()` が、on-demand コレクションでは保存済み行を直接使えないため、チャンネル直下の最新ページを IndexedDB の控えから復元します。シェルと本文で復元元が異なります。

- `src/components/offline/OfflineProvider.tsx` が起動時に名前空間を決めて DB を開き、その DB を使うコレクションを `CollectionsProvider` 経由で子へ渡します。dev / preview は固定ユーザー、OIDC は解決済みの現在ユーザー、解決前やオフラインは最後に使ったユーザーの名前空間を使います（`src/lib/offline/identity.ts`）。オンラインで本人を確認できない場合は前ユーザーのキャッシュを開きません。
- DB は `@tanstack/browser-db-sqlite-persistence` の wa-sqlite + OPFS で、ユーザーごとに物理的に分けます（ファイル名は `zoo-offline-` 接頭辞と名前空間のハッシュ。wa-sqlite の OPFS VFS は長いファイル名で開けないため id をそのまま使いません）。スキーマ不一致時は `sync-present-reset` で同期します。OPFS / Web Worker が使えない環境ではメモリのみにフォールバックし、アプリは起動します。
- Service Worker（`public/sw.js`）がアプリシェルと遅延チャンク・フォント・OPFS worker をキャッシュします。オフライン再読み込みでは `/index.html` → `/` の順にキャッシュ済みのシェルを探し、両方なければ `public/offline.html` へフォールバックします（`public/sw.js` の `handleNavigation()` と `refreshNavigation()`）。新しい配備のシェル更新は起動時に一度だけ確認し、入力中のフォームを消さないようにしています。
- ログアウト時はそのユーザーの DB（`-journal` / `-wal` を含む）と IndexedDB の控えを削除します。削除に失敗しても次回起動で再試行し、別ユーザーのキャッシュを開かないようにします。
- `pnpm test:e2e:offline` がオフライン再読み込み後のシェルとメッセージ復元、ユーザーごとの DB 分離を確認します。永続化の制御層は Storybook の監査対象外で、`src/components/offline/OfflineProvider.browser.test.tsx` で確認します。

### 言語設定（i18n）

日本語・英語・繁體中文・简体中文・廣東話（繁体字表記）・韓国語・朝鮮語（文化語）に対応しています。左レールの言語メニューで切り替えます。ログアウト・ログインエラー画面にも同じメニューがあります。

- 言語は `localStorage` の `zoo-locale` に保存します。保存値がない場合は `navigator.languages` の順に対応言語を探し、該当がなければ日本語を使います。ストレージが無効でもセッション内の切り替えは可能です。
- ロケールは `ja` / `en` / `zh-Hant` / `zh-Hans` / `yue` / `ko-KR` / `ko-KP`。ブラウザの `zh-TW`・`zh-HK`・`zh-MO` は繁体字、`zh-CN`・`zh-SG` は簡体字、`yue-HK` は広東語に解決します。広東語は中国語の地域指定だけからは推定しません。`ko`・`ko-KR` は韓国語、`ko-KP` だけが朝鮮語（文化語）に解決します。朝鮮語の照合順（`Intl.Collator`）は韓国語と同じです。
- 翻訳は `src/lib/i18n/locales/*.json` にまとめ、React コンポーネントでは `useTranslation()` を使います。全言語でキーと補間変数を一致させるテストがあります。文言追加時は7ファイルを更新してください。
- 日付は選択した言語と端末のタイムゾーンに合わせて `Intl.DateTimeFormat` で表示します。件数は i18next の複数形ルールを使います。ユーザーが入力したメッセージ・名前や、外部サービスから返された診断文は翻訳しません。
- SPA のプリレンダリングとハイドレーション中は、言語に依存しないスケルトンを表示します。`head` の初期化スクリプトが `html.lang` を先に設定し、クライアントでは保存値／ブラウザ言語で i18next を初期化してから画面をマウントするため、日本語が一瞬表示されることを防ぎます。
- i18next が言語の唯一の情報源です。`useLocale()` は `useSyncExternalStore` で同じインスタンスを購読し、翻訳ヘルパーにもその `t` を渡します。言語とテーマのストレージ読み書き・別タブとの同期は `src/lib/storage.ts` を共有し、`localStorage.clear()` も反映します。
- 自前の OIDC エラーは理由コードから7言語で表示し、エラー中に言語を切り替えても翻訳を更新します。外部 IdP が返した説明は元の文言を保ちます。新着の読み上げ文は通知を出した時点の言語で固定し、言語変更だけでは再通知しません。

```bash
pnpm test:e2e:i18n    # 本番ビルド + Playwright。バックエンド不要
```

ブラウザテストは開発用5173番とは別の4174番で実行します。ローカルではビルド後にプレビューを起動し、CIでは直前のビルドを再利用します。

ブラウザテストには Playwright の Chromium が必要です（`pnpm exec playwright install chromium`）。各言語の自動判定・ダイアログ・切り替え・再読み込み後の保存値を確認します。既存のスモーク / OIDC テストは表示言語を日本語に固定しています。

### テストの入口

- `pnpm check` … format / lint / typecheck。CI（`.github/workflows/ci-frontend.yaml`）が `frontend/**` の変更（`main` への push / PR）で `pnpm check`・`pnpm build`・`pnpm test` を実行します。
- `pnpm test` … vitest のユニットテスト（`src/**/*.test.ts`）。数値・真偽値の文字列変換、timestamp 正規化、設定解決、REST アダプタの `{ data, txid }` 注入と剥がし・エラー化、`txid` ハンドシェイク、Shape の URL / ヘッダ / live query を担保します。
- `pnpm test:browser` … ブラウザ上で動く Vitest。永続化の制御層などを確認します。
- `pnpm test:e2e` と `pnpm test:e2e:*` … Playwright の E2E。起動条件的な詳細は後述の「テスト・品質」と「Storybookの構成」を参照してください。
- `pnpm storybook` / `pnpm check:storybook` / `pnpm test:storybook` … カタログの起動・分類監査・Chromium 上の play / a11y。詳細は「Storybookの構成」を参照してください。

## 必要環境

- Nix devShell（Node.js 22 と pnpm を供給）。リポジトリルートで `nix develop`。
- バックエンド / Electric を動かす場合はルートで `make db-up && make migrate`。

## セットアップ

```bash
make crypto-wasm   # E2EE の wasm（crates/zoo-crypto-wasm/pkg）を作る。リポジトリルートで実行
cd frontend
pnpm install
```

`@zoo/crypto-wasm` は `crates/zoo-crypto-wasm` への `link:` 依存です。`pkg/` は Git に入れないので、`pnpm check`、`pnpm test`、`pnpm test:browser` の前に `make crypto-wasm` で作ります（Rust の `wasm32-unknown-unknown` と、`crates/zoo-crypto-wasm/Cargo.toml` の `wasm-bindgen` と同じ版の `wasm-bindgen-cli` を使います。どちらも `nix develop` に入っています）。`crates/zoo-crypto` を変えたら作り直してください。`make frontend-check`、`frontend-test`、`frontend-build` は先に作ります。

## コマンド

Vite+ の `vp` をそのまま使います（`package.json` の scripts も同じ）。

```bash
pnpm dev              # 開発サーバ (http://localhost:5173)
pnpm build            # 本番ビルド (SPA モード: dist/client/ に index.html と静的アセットを出力)
pnpm start            # ビルドのプレビュー (vp preview)
pnpm preview          # ビルドのプレビュー
pnpm check            # format / lint / typecheck
pnpm lint             # lint のみ
pnpm test             # vitest (Vite+ 内蔵)
pnpm test:e2e         # Playwright スモーク (要: DB + Electric + バックエンド)
pnpm test:e2e:layout  # 編集・モバイル表示領域のレイアウト回帰テスト (バックエンド不要)
pnpm test:e2e:oidc    # OIDC PKCE E2E (モック IdP + 本番同等設定のバックエンド)
pnpm test:e2e:csp     # nginx の本番 CSP をブラウザで検証 (要: ビルド済みフロントコンテナ)
pnpm generate-routes  # ルートツリーを再生成
```

認証E2Eは、HTTP/1.1の接続上限でShapeのSSEがREST送信を塞がないよう、
一時的な自己署名証明書を使うHTTP/2プロキシ経由でバックエンドへ接続します。
`openssl` が必要です。プロキシは既定で8443番を使い、`E2E_HTTP2_PORT` で変更できます。
`VITE_API_BASE_URL` はプロキシから接続するバックエンドのURLです。

ルートの Makefile からも実行できます: `make frontend-dev` / `frontend-build` / `frontend-check` / `frontend-test` / `frontend-e2e`。

開発サーバは Electric（ポート 3000）と衝突しないよう **5173** で起動します。

本番ビルドは TanStack Start の SPA モード（`spa: { enabled: true }`）を使い、プリレンダリングされた SPA シェル（`dist/client/index.html`）とクライアントアセットを `dist/client/` に出力します。コンテナでの静的配信は [インフラの README](../infra/README.md#ステージング) を参照してください。

## 構成

- `src/routes` … ファイルベースルーティング（TanStack Router）
- `src/components/ui` … shadcn/ui（Base UI）コンポーネント
- `src/hooks` … live query の購読フック（`useRealtime.ts`）
- `src/lib` … ユーティリティとデータアクセス層（`config.ts` / `auth.ts` / `api/` / `electric/`）
- `src/styles.css` … Tailwind v4 と shadcn テーマ
- `components.json` … shadcn 設定（`pnpm dlx shadcn@latest add <name>` で追加）

`src/routeTree.gen.ts` は生成物です。追跡はしますがフォーマッタ対象からは除外しています（`.prettierignore`）。

## 絵文字ピッカー

リアクションの絵文字ピッカーは [frimousse](https://frimousse.liveblocks.io)（headless）で描画し、絵文字データは `public/emojibase-data` から自前配信します。実行時に CDN へはアクセスしません。

- 配信するロケールは `en` / `ja` / `ko` / `zh-hant` の4つです。アプリのロケールとの対応は `src/lib/emoji/locale.ts` の中央マップで解決します（`zh-Hans` と `yue` は繁体字データ、`ko-KR` と `ko-KP` は `ko` データを使い、未対応は `en`）。
- frimousse は初回に選択ロケールの `data.json` と `messages.json` を取得し、localStorage にキャッシュします。2回目以降のオープンではキャッシュを使います。
- どの言語でも英語・日本語・韓国語・中国語とピンインで検索できるよう、`resolveMultilingualEmojiData`（`src/lib/emoji/resolver.ts`）が `search-index.json`（約 940KB、gzip で約 290KB）も1回取得し、各絵文字の検索用タグに足します。取得に失敗したときは選択ロケールのデータだけで検索します。足したタグは localStorage に保存しません（frimousse が保存するのは `defaultEmojiDataResolver` が取得した元データだけ）。
- 表示する絵文字は `SUPPORTED_EMOJI_VERSION`（`src/lib/emoji/data.ts`、現在は 16）までに制限します。バックエンドの `emojis` クレートが Unicode 16.0 までしか検証しないためで、どちらかを更新するときは両方の上限を揃えてください。
- recents は localStorage の `zoo-recent-emoji` に最大16件、新しい順で保存します。
- データは `emojibase-data`（devDependency）からコピーします。バージョンを更新したら次を実行し、差分をコミットしてください。

```bash
pnpm sync:emojibase
```

`public/emojibase-data/LICENSE` に上流の MIT ライセンスを同梱しています。

## テスト・品質

主要なデータ変換と書き込みフローを vitest で担保し、`vp check`（format / lint / typecheck）とテストを CI で回します。

- ユニットテスト（vitest / `src/**/*.test.ts`）… 数値・真偽値の文字列変換（`schema.test.ts`）、timestamp 正規化（`timestamps.test.ts`）、設定解決（`config.test.ts`）、REST アダプタ `{ data, txid }` の注入と剥がし・エラー化（`api/client.test.ts`）、`txid` ハンドシェイク（`txid.test.ts` / `writes.test.ts`）、Shape の URL / ヘッダ / live query（`electric/*.test.ts`）。
- CI … `.github/workflows/ci-frontend.yaml`。`frontend/**` の変更（`main` への push / PR）で `pnpm check`・`pnpm build`・`pnpm test` を実行します（バックエンドの CI とは別ワークフロー）。

### デザインシステムの lint（`@shadcn/lint`）

`pnpm check`（または `pnpm lint`）は、Oxlint 経由で `@shadcn/lint` の 6 ルールも実行します。設定は `vite.config.ts` の `lint` ブロックにあります。

| ルール                       | 目的                                                   |
| ---------------------------- | ------------------------------------------------------ |
| `shadcn/no-restyle`          | `className` で UI コンポーネントの色・形を上書きしない |
| `shadcn/no-raw-colors`       | テーマに宣言した色トークンを使う                       |
| `shadcn/no-arbitrary-values` | 任意値よりテーマのスケールを使う                       |

ほかに `no-inline-styles` / `no-unknown-classes` / `require-static-classes` も有効です。エラーには「何を使うべきか」が表示されます。

- 色は `src/styles.css` の `@theme` に宣言したトークン（`brand` / `mention` / `success` / `warning` / `destructive` など）を使う。状態色は既存の意味を保つこと。
- `Button` の色・形は `variant`（例: `success-solid` / `destructive`）と `shape="circle"`、`size` で表現する。呼び出し側で `className` を足さない。
- 呼び出し側が責任を持つ項目（例: `Input` のタイポグラフィ、`DialogContent` の余白）は `no-restyle` の `contracts` で許可しています。新しい contract や `allow` を増やすときは、デザイン上の根拠を PR 本文に書く。
- `src/components/ui/**` はコンポーネント自身が内部スタイルを所有するため、`no-restyle` / `no-arbitrary-values` / `require-static-classes` は対象外です（`no-raw-colors` などは有効）。
- テスト・Story のハーネス（`*.stories.tsx` / `*.browser.test.tsx` / `src/test/vrt/**` / `e2e/**`）も一部ルールを対象外にしています。
- スモーク（Playwright / `e2e/smoke.spec.ts`）… 起動 → チャンネル作成 → メッセージ送信 → Electric 反映までを実ブラウザで確認します。`pnpm test` には含めず `pnpm test:e2e` で明示実行します。前提として DB + Electric + バックエンドが必要です。

```bash
make db-up && make migrate        # PostgreSQL + Electric
make backend-dev                  # DEV_AUTH_ENABLED=true (http://localhost:8000)
cd frontend && pnpm test:e2e      # 起動 → 送信 → 反映
```

`pnpm test:e2e` はフロントエンド（proxy モード）だけを起動し、バックエンドは既存のものを再利用します。現在ユーザー（`VITE_DEV_USER_ID`、既定 `...0001`）が未作成なら `POST /api/users` でスモーク用に作成します。

### OIDC ログイン E2E（#50）

ログイン済みのセッションは同じタブの `sessionStorage` に保持し、再読み込み時に復元します。再認可が必要になるまでは、IdP への画面遷移を省きます。更新トークンと更新セッションの識別子も保存し、再読み込み後やアクセストークン期限切れ後も自動更新します。更新は同一オリジンのSharedWorkerへ集約し、複製タブにも最新の更新結果を共有します。Worker終了後はIndexedDBから最新状態を復元します。更新結果が不明な場合は古いトークンを再送せず再認可します。詳しくは[認証ワーカーの設計](../docs/oidc-refresh.md)を参照してください。認証設定の変更・ログアウト・セッション失効時は保存情報を破棄し、再ログインします。タブを閉じるとそのタブのsessionStorageは消えますが、IndexedDBの更新状態は残ります。ログアウト・セッション破棄時に資格情報を消し、24時間以上使われていない行も次回Worker利用時の掃除で終了状態にします。SharedWorker・Web Locks・IndexedDBが利用できない環境では更新トークンを保存しません。

`e2e/oidc.spec.ts` は、外部依存なしのモック IdP（`e2e/mock-idp-server.ts`）と本番同等の認証設定を使い、**未ログイン → authorize → ログイン → Bearer で REST/Shape → チャンネル作成 → メッセージ送信 → Shape 反映**を通しで確認します。自分のメッセージにだけ編集/削除が出ること、ログアウト → 別ユーザーでの再ログイン、参加/退出も含みます。

Playwright がモック IdP（既定 `http://127.0.0.1:4100`）とフロントの dev サーバ（`http://localhost:5173`）を起動します。バックエンドは先に起動しておいてください（既定のスモークとは別設定です）。

```bash
make db-up && make migrate
cd backend
DEV_AUTH_ENABLED=false ENVIRONMENT=production \
  OIDC_ISSUER=http://127.0.0.1:4100/oauth2/openid/zoo OIDC_AUDIENCE=zoo \
  CORS_ALLOWED_ORIGINS=http://localhost:5173 \
  DATABASE_URL='postgres://root:password@localhost:5432/zoo?sslmode=disable' \
  ELECTRIC_URL=http://127.0.0.1:3000 \
  cargo run
# 別ターミナル
cd frontend && pnpm test:e2e:oidc
```

- `OIDC_ISSUER` は `playwright.oidc.config.ts` が起動するモック IdP の issuer と一致させます。ポートを変える場合は `MOCK_IDP_PORT` と `OIDC_ISSUER` の両方を揃えてください。
- `ELECTRIC_URL` は `localhost` ではなく `127.0.0.1` を指定します（Docker の公開ポートが IPv4 のみの環境で `::1` に解決されるのを避けるため）。
- テストはモック IdP の `/__mock/user` でユーザーを切り替えます。テスト終了後にモック IdP とフロントは Playwright が停止します。
- モック IdP は起動のたびに署名鍵を生成します。テストを再実行する際は、古い JWKS キャッシュが残らないようバックエンドも再起動してください。
- CI では `.github/workflows/ci-frontend-auth-e2e.yaml` が PostgreSQL + Electric + Atlas migration + Rust を起動して同じテストを実行します。重いため `pnpm test` / `pnpm test:e2e` からは分離しています。

CSP は `infra/frontend-nginx.conf`（コンテナ起動時に envsubst）で設定します。`script-src` はビルド時のインライン script ハッシュ、`connect-src` は同一オリジン + `CSP_CONNECT_SRC`（IdP のオリジン）+ `CSP_STORAGE_SRC`（添付ファイル用ストレージのオリジン。カスタムドメイン運用時は `https://attachments.zliusercontent.com`）、`img-src` は Emoji Kitchen 用の `https://www.gstatic.com`、Composer の添付プレビュー用の `blob:`、`CSP_STORAGE_SRC`・`CSP_APP_IMAGE_SRC`・`CSP_AVATAR_SRC`を含みます。`CSP_AVATAR_SRC`はOIDCアイコンの配信元で、既定は`https://avater.zliusercontent.com`です。別のIdPを使う配備では、その画像配信元のHTTPSオリジンを実行時に指定します。ステージングでの設定・確認手順は [インフラの README](../infra/README.md#csp) を参照してください。

`pnpm test:e2e:csp` は、nginx が配信するビルド済みのフロントに対して Playwright を 1 本実行し、`securitypolicyviolation` が 0 件であること、テーマ初期化 script が実行されること（`<html>` の `colorScheme`）を確認します。既定では `http://127.0.0.1:3100` を見るため、CI の `container` ジョブと同じようにイメージを起動してから実行してください。

```bash
# リポジトリルートで
docker build -f infra/Dockerfile-frontend-deploy -t zoo-frontend:stg .
docker run --rm -d --name zoo-frontend-csp-test -p 127.0.0.1:3100:3000 zoo-frontend:stg
cd frontend
E2E_BASE_URL=http://127.0.0.1:3100 pnpm test:e2e:csp
docker rm -f zoo-frontend-csp-test
```

VRTはOSのフォントに依存しないよう、テスト内でGeistと固定バージョンのNoto Sans JPを読み込みます。文字のサブピクセル描画・ヒンティングも固定し、描画後にフォントの読み込みを待って比較します。フォント・表示を意図して変更した場合は、差分を確認して基準画像を更新してください。

## 入力中表示

チャンネルとスレッドごとに、他の参加者の入力中表示を出します。入力内容は送らず、認証済みの `GET /api/channels/{id}/typing` と `POST /api/channels/{id}/typing` で短命な状態を扱います。スレッドでは `parent_id` にルートメッセージIDを指定します。

- 入力通知は最大2秒に1回、表示の取得は1.5秒間隔です。
- 3秒間入力がない場合、送信・入力欄のクリア・フォーカス離脱・画面移動・タブ非表示時に停止します。停止通知が届かなくても、サーバー側で6秒後に失効します。
- 本人は一覧から除外し、複数タブで入力していても1人として表示します。参加者だけが取得・更新でき、アーカイブ済み・削除済みのチャンネルは対象外です。
- 状態はPostgreSQLの `typing_presence` テーブルで共有します。同じDBを参照する複数のバックエンドインスタンスで動作し、セッション固定は不要です。有効期限にはDBの時刻を使い、インスタンス間の時計のずれを避けます。
- デプロイ時は先に `20261023000000_create_typing_presence.sql` を適用してください。期限切れの行は取得対象から即座に除外し、入力通知時に最大1,000件ずつ削除します。Redisや追加の常駐プロセスは不要です。

## 送信と既読更新の順序

メッセージは保存前から楽観表示しますが、既読更新は書き込みトランザクションの確定後に行います。`useWriteRunner` は処理中の件数を保持し、並行する書き込みがすべて完了するまで `MessageList` の既読更新だけを保留します。表示や未読境界は維持します。保存・同期失敗時もロールバック後に保留を解除し、存在しない楽観行を既読APIへ送らないようにします。最下部・フォーカス・タブ表示の条件は既存の判定を使用します。

## MFM 本文表示

通常メッセージとスレッド返信に MFM を使用できます。
構文一覧・引数・動作制限は [MFM 対応表](../docs/mfm.md) を参照してください。
例: `$[jelly 🎉]`、`$[spin.x,speed=2s 🐱]`、`$[ruby 漢字 かんじ]`、`\(x^2\)`。
OS の `prefers-reduced-motion` 設定ではアニメーションを停止します。

解析は [mfm-js](https://github.com/misskey-dev/mfm.js)（MIT）、数式は
[KaTeX](https://katex.org/)（MIT）、コードの色付けは
[highlight.js](https://highlightjs.org/)（BSD-3-Clause）を使用します。
描画処理とアニメーションは本プロジェクトの独自実装です。
間接依存と絵文字データを含めた著作権表記・ライセンス全文を
[`public/licenses`](public/licenses) に同梱し、配信時も `/licenses/README.txt` から確認できます。

## Zoo設定とメッセージ操作

左レール上部のZooアイコンから「Zooについて」を開けます。Web版のバージョンとして、ビルド元コミットの先頭12桁を表示します。

左レールの歯車から `/settings` を開けます。表示・通知の設定、App連携、Zooメンバーの一覧をまとめています。App連携は検索・絞り込みに対応し、「使う」と「管理・開発」を切り替えます。対象チャンネルを選択でき、所有者または管理者は確認後にAppを削除できます。

管理者はメンバー一覧から管理者権限を付与・解除できます。`PATCH /api/users/{id}/admin` は管理者だけが実行でき、最後の管理者と開発用の固定管理者の解除を拒否します。権限変更はトランザクション内で直列化します。DMのメッセージ削除権限は投稿者本人のままです。

通常チャンネルの管理者・作成者は、投稿・スレッド返信の操作メニューから削除できます。公開チャンネルは「探す」から未参加のまま閲覧し、送信欄の参加ボタンで参加できます。スレッドの開閉はURLと履歴に反映され、戻る・進むで復元します。

インストールしたPWAでは、起動時の画面まで戻った後の「戻る」を吸収し、その画面に留まります。境界はウィンドウのセッション単位で保持し、通知や認証コールバックによる文書読み込みでは再挿入しません。利用者が選んだ「起動地点に留まる」動作として扱い、繰り返しの操作で通知が重ならないよう、吸収時のトーストや読み上げ通知は出しません。アプリ内の「戻る・進む」と通常タブの履歴操作は維持します。OSによるアプリ終了や履歴を複数件飛び越す操作は抑止の対象外です。

共通のInput・Textareaと検索コントロールには、自動入力を抑える属性を設定しています。ブラウザやパスワード管理拡張の挙動によっては、これらの指定が無視される場合があります。

## ブランドカラーとアイコン

[Zli Brand Guidelines](https://zli.works/brand) の Yellow `#FFD600` をアクセントに使い、白黒の基礎色を `#FFFFFF` / `#000000` として、`src/styles.css` で明暗の共通トークンを定義しています。ライトテーマのメイン背景は白を維持し、サイドバーなどのカラムの黄色味はごく薄くし、補助色・境界線・ダークテーマの選択背景はニュートラルなグレーに揃えます。黄色のボタンには黒い文字を使い、本文のリンクはテーマに合わせた青を専用の `--link` トークンで管理します。

アイコンの正本は `public/icons/zoo.svg` です。提供されたSVGのパスを維持し、アプリ左上・ウェルカム画面・SVG faviconで同じファイルを参照します。PWA / Apple用PNG、PNG favicon、ICO、通知用の単色バッジもこのSVGから生成します。変更時は `pnpm exec playwright install chromium` の後、`pnpm generate:icons` を実行し、生成ファイルもコミットしてください。PWAのマークはmaskableアイコンの中央80%の円内に収め、OSによる切り抜きに備えています。通知バッジはAndroidの描画仕様に合わせ、同じ図案の透過・白一色を使います。

## Storybookの構成

分類と追加基準は [storybook-coverageスキル](../.agents/skills/storybook-coverage/SKILL.md) にまとめています。

- **Foundations**: 実際の色・文字・余白の規則。
- **Components**: 再利用する入力、確認ダイアログ、設定セクションなど。
- **Features**: チャット、チャンネル、DM、App、メンバー管理の表示状態。
- **Pages**: 画面全体の構成とレスポンシブ表示。
- **Flows**: 確認を伴う権限変更や、失敗からの再送信などの操作手順。

通常・空・読込中・失敗・権限差・長い内容など、確認したい状態をStory単位にします。
テーマと対応7言語はツールバーで切り替えられます。実部品を共有し、データはローカルコレクションとMSWで再現します。

```bash
pnpm storybook         # 開発用カタログ
pnpm check:storybook   # 分類・対象一覧・必須状態の欠落
pnpm test:storybook    # Chromium上のplayとa11y
pnpm build-storybook  # 静的カタログ
```

[対象一覧](.storybook/coverage.json) はUIモジュールと確認先Storyを対応づけます。新しいUI・Storyの未登録や必須状態の削除をCIで検出します。
認証の外部リダイレクト、プレビュー専用ビルド、永続化など6つの制御層・環境依存箇所は理由と別テストを記録し、Storyで確認済みの数には含めません。
この監査は見た目の正しさや全状態の網羅を保証するものではありません。

## Storybook の PR プレビュー

PR の作成・更新時に Storybook を Cloudflare Pages へ公開します。
PR のデプロイ欄または Actions の実行サマリーから開けます。
初回設定と運用は [Storybook の PR プレビュー](../docs/storybook-preview.md) を参照してください。

## 履歴（実装の経緯）

以下は実装当時の作業区分と判断の記録です。現在の設定・手順は「現在の設計」を参照してください。

### データアクセス層（Phase 1）

Electric の Shape 購読と REST 書き込みを分離し、接続先を設定だけで切り替えられるようにしています。

- `src/lib/config.ts` … `resolveConfig()` / `buildShapeUrl()`。`VITE_SHAPE_MODE` が `direct` なら Electric（`VITE_ELECTRIC_SHAPE_URL`）、`proxy` ならバックエンドの Shape プロキシ（`VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`）を参照します。**既定は `proxy`**（開発・本番とも）。不正な値は例外にします。開発で `proxy` を使うときは現在ユーザー（`X-Dev-User`）が必要なため `VITE_DEV_USER_ID` を必須にします（未設定は起動時エラー、OIDC 設定時は Bearer を使うため不要）。**`direct` は Electric を認可なしで直接叩く開発専用**で、プライベートチャンネルのフィルタも効きません。
- `src/lib/auth.ts` … `getToken()` のみのアダプタ。`setTokenProvider()` で IdP を差し替えられます（既定は未認証で `null`）。`buildAuthHeaders()`（`api/client.ts`）を使うと REST 側の fetch に同じ `Bearer` / `X-Dev-User` を渡せます。
- `src/lib/api/client.ts` … REST クライアント。`Bearer`（`getToken()`）と `X-Dev-User`（`VITE_DEV_USER_ID`、開発時のみ）を注入し、書き込み系の `{ data, txid }` エンベロープを取り出します（`unwrapEnvelope()`）。書き込みは Phase 3 以降で、Phase 2 の読み取りは Shape 購読のみを使います。
- `src/lib/electric/` … 行型とスキーマ定義。integer は文字列で届く前提で `coerceElectricRow()` が `Number()` 変換します（`int8` / `numeric` は精度のため文字列のまま）。timestamp は Shape が `2026-10-15 14:32:44.183`（空白区切り）、REST が `T` 区切りの文字列で、どちらも UTC の naive 値です。`normalizeUtcTimestamp()` / `parseUtcTimestamp()` で UTC の ISO 文字列・`Date` に正規化できます。

環境変数は `frontend/.env.example` を参照してください。

### リアルタイム購読（Phase 2）

`make db-up && make migrate` の後、開発サーバ（既定 `proxy`。バックエンドと `VITE_DEV_USER_ID` を用意。下記 Phase 5）で開くと、チャンネル・メッセージ・投稿者名がリアルタイム表示されます。表示は Phase 4 の画面（`/` と `/channels/$channelId`）で行います。

- `@tanstack/electric-db-collection` で Electric Shape を購読し、`@tanstack/react-db` の `useLiveQuery` で表示します。書き込みと楽観更新は Phase 3、画面は Phase 4（いずれも下記）です。
- `src/lib/electric/collections.ts` … `channels` / `users` / `channel_user` / `messages` のコレクション。`messages` は `channel_id` 単位で生成してキャッシュします。ShapeStream はブラウザ API に依存するため、モジュール直下では作らず client コンポーネント / フックから `getCollections()` を呼びます（SSR では購読しません）。
- `src/lib/electric/query.ts` … `buildShapeUrl()` に渡す Shape クエリ。`direct` では Electric の subset プロトコル（`where` + `params[1]`）、`proxy` では `channel_id` を渡します（`table` は direct では `buildShapeUrl()`、proxy ではバックエンドが付与）。
- `src/lib/electric/headers.ts` … ShapeStream 用 fetch。REST と同じ `Bearer` / `X-Dev-User` を「値があるときだけ」付与します。常時付与すると開発時の Electric 直結で `authorization` が CORS の preflight に弾かれるためです。
- `src/lib/electric/messageFeed.ts` … `messages ⋈ users` の live query。投稿者名は非正規化せず `users` Shape を結合して表示し、`deleted_at` をクエリ側で除外します。
- `src/hooks/useRealtime.ts` … 上記コレクションを購読するフック（チャンネル一覧・メッセージ・メンバー・ユーザー）。

### 書き込みと楽観更新（Phase 3）

画面から実行できる書き込みは REST、表示は Electric Shape で、書き込み後に `txid` を突き合わせて楽観状態を確定させます。

- `src/lib/electric/txid.ts` … `syncTxid()` と `changeMessageMatcher()`。REST の `{ data, txid }` から数値化した `txid` を `collection.utils.awaitTxId()` に渡します（EDC の `Txid` は number）。`VITE_TXID_MODE` で `awaitTxId`（既定）/ `awaitMatch`（operation と行 id で照合）/ `skip`（待機しない）を切り替えられます。
- `src/lib/electric/writes.ts` … コレクションごとの `onInsert` / `onUpdate` / `onDelete`。REST を呼び、失敗時は throw して TanStack DB に楽観状態をロールバックさせます。
  - messages: 送信 `POST /api/messages`、編集 `PATCH /api/messages/{id}`、論理削除 `DELETE /api/messages/{id}`。
  - channels: 作成 `POST /api/channels`、更新 `PATCH /api/channels/{id}`。
  - channel_user: 参加 `POST /api/channels/{id}/join`、メンバー追加 `POST /api/channels/{id}/users`、退出 `POST /api/channels/{id}/leave`。`/leave` は認証ユーザー自身の行しか消せないため、現在のユーザー（`config.devUserId`）以外の行の削除は throw して楽観状態を戻します（メンバー削除 API ができるまでの暫定）。
- 論理削除は Electric 上では `update`（`deleted_at` の更新）として流れます。そのため楽観更新も `update(deleted_at)` に揃え、`delete→insert` の再出現（ちらつき）を避けています。コレクションは全行を持ち、`messageFeedQuery` が `deleted_at` を除外します。
- channel_user の書き込みは channel_user コレクションで `txid` を待ちます。proxy の channels Shape はバックエンドがメンバーシップで絞るため、参加/退出（join/leave）に応じて channels 側の行が move-in / move-out します。move-in / move-out 自体は同じトランザクションの `txid` で配信されますが、書き込みハンドラが待つのは channel_user コレクションの `txid` です（チャンネル一覧は続けて更新されます）。
- `awaitTxId` は対象コレクションが live query で購読されている必要があります（画面に出しておく）。未購読だとタイムアウトします。なお `awaitTxId` / `awaitMatch` のタイムアウトでも楽観状態はロールバックされ、UI には操作失敗と出ますが、サーバ側の書き込みは成功しているため少し遅れて Electric から同じ行が届き表示は戻ります。「同期待ち」と「失敗」を分けた表示は今後の課題です。
- `src/lib/electric/collections.ts` … ShapeStream の `transformer` で Electric の行を型へ正規化します（integer は number、boolean は `"true"` / `"false"` 文字列を真偽値へ）。`"false"` が truthy になるのを防ぎます。

### 画面（Phase 4）

shadcn（Base UI）のテーマのみで最小の画面を組んでいます。細かい Tailwind の作り込みは行いません。

- ルート `/` … チャンネル一覧・作成・参加/退出・メンバー表示（`src/routes/index.tsx` → `src/components/ChannelDirectory.tsx`）。各チャンネルは `Link` でメッセージ画面へ遷移します。
- ルート `/channels/$channelId` … メッセージ一覧・送信・編集・論理削除・投稿者表示（`src/routes/channels/$channelId.tsx` → `src/components/chat/messages/MessagePane.tsx`）。
- ルート `/realtime` … 検証 UI は上記の実ルートへ移行しました。ここは接続モード（Shape / txid / 開発ユーザー）を表示する最小画面です（`src/components/SyncStatus.tsx`）。
- `src/hooks/useWriteRunner.ts` … `collection.insert()` などの楽観更新トランザクションを `when("settled")` まで待ち、失敗を UI のエラーメッセージにまとめます。
- 編集・削除は自分のメッセージにのみ表示します（バックエンドは投稿者のみ許可するため）。開発で `proxy` モードを使うときは `VITE_DEV_USER_ID` が必須です（未設定は起動時エラー、OIDC 設定時は Bearer を使うため不要）。`frontend/.env.example` の例にならい設定してください。
- 非公開チャンネルは参加者だけに説明・メンバー・メッセージを表示します。`proxy` の Shape はバックエンドが認可で絞るため未参加者には届きませんが、UI 側でも `isMember` で隠します。`direct` は認可せず全件配信するため UI 側のガードだけが頼りです。`/channels/$channelId` を URL 直開きしても未参加ならメッセージは出しません。説明はチャンネルのヘッダーに表示し、メンバーのダイアログには出しません。
- メンバー追加フォームはバックエンドの `add_channel_user` と同じ条件（作成者は常に、非公開は作成者のみ、公開は作成者またはメンバー）でのみ表示します。
- `ChannelMemberPanel` はチャンネルごとに置きますが、`channel_user` の Shape は `getCollections()` が 1 本だけ作り、パネルはそのコレクションを購読します。`users` は全パネルで同一クエリのため TanStack DB 内部でプール共有されます。同じ channel の `useChannelMembers`（画面とパネル）もプールされ共有されるため、購読はチャンネル数に対して増えません。
- 既存のガードを踏襲します。重複メンバーの追加は UI 側で弾き、現在のユーザー以外の退出は書き込みハンドラが throw してロールバックします。メンバー読み込み中は重複判定が効かないため参加/追加ボタンを無効化します。

### バックエンド経由への切替（Phase 5）

Shape の接続先をバックエンドの Shape プロキシ（`/api/shapes/*`）に切り替え、`awaitTxId` を既定で有効にしました（`VITE_TXID_MODE=awaitTxId`）。既定の `VITE_SHAPE_MODE` は `proxy`（開発・本番とも）です。

#### セットアップ

```bash
make db-up && make migrate        # PostgreSQL + Electric
make backend-dev                  # DEV_AUTH_ENABLED=true cargo run (http://localhost:8000)
cp frontend/.env.example frontend/.env
cd frontend && pnpm dev           # http://localhost:5173
```

`frontend/.env` では `VITE_SHAPE_MODE=proxy` と `VITE_DEV_USER_ID`（現在ユーザー。`X-Dev-User` として送信。DB の `users` に存在する id）を設定します。**開発で `proxy` のときは `VITE_DEV_USER_ID` が必須**で、無いと起動時エラーになります（OIDC 設定時は Bearer を使うため不要、`.env` のコピー忘れに注意）。バックエンドの既定ユーザーは `DEV_DEFAULT_USER_ID`（既定 `00000000-0000-0000-0000-000000000001`）で、`X-Dev-User` が無いときはこのユーザーとして扱われます。

#### 経路とポイント

- 読み取り: `proxy` は `VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`（既定 `/api/shapes`）を購読します。バックエンドが `table` / `where` を確定し、サーバ側で `ELECTRIC_SECRET` を付けて Electric へ転送します。`messages` / `channel_users` はクエリで `channel_id` を渡します。
- 認証: REST と Shape の両方で同じ `X-Dev-User`（と `Authorization`）を受け付けます。バックエンドは `DEV_AUTH_ENABLED=true` のときだけこのヘッダを受け付け、それ以外は 403 です。CORS はバックエンドが `CORS_ALLOWED_ORIGINS` の許可オリジンに限定し（`ENVIRONMENT=production` 以外で未設定なら `http://localhost:5173`, `http://127.0.0.1:5173`、本番で未設定なら空）、メソッドとヘッダも既知のもの（ヘッダは `Authorization`、`X-Dev-User`、`Content-Type`、`Accept`、E2EE の直前の再認証の `X-Zoo-Reauth`）だけを許可しつつ `electric-*` を公開します。`direct` は Electric の CORS が `X-Dev-User` / `Authorization` を許可しないため認証ヘッダを付けません（`collections.ts` の `shapeAuthSource`）。
- 認可: バックエンドの channels Shape は `is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = ...)` でメンバーシップに絞ります。未参加の非公開チャンネルは Shape・メッセージのどちらも届きません（存在隠蔽のため 404）。参加/退出で channels 側の行が move-in / move-out します。
- `channel_users` のサブクエリ経路（`electric#3143`）: Electric 1.8.1 を `ELECTRIC_FEATURE_FLAGS=allow_subqueries` で動かしており、`WHERE ... IN (SELECT ... WHERE user_id = $1)` の形がそのまま通ることを実測しました（`params[1]` により move-in / move-out も配信）。`$1` をリテラルへ埋め込む回避策は不要でした。
- 書き込み: すべて REST（`{ data, txid }`）で行い、`txid` を `awaitTxId` で Electric の配信と突き合わせます。失敗時は楽観状態がロールバックします。

### テスト・品質（Phase 6）

vitest と `vp check` による品質担保を整え、CI で回すようにしました。現在のテストの入口と詳細は「テストの入口」と「テスト・品質」を参照してください。

### 完了状況（記録時点）

Phase 0（足場）・Phase 1（型・データアクセス層）・Phase 2（読み取り・live query）・Phase 3（書き込み・楽観更新）・Phase 4（UI: shadcn テーマのみ）・Phase 5（バックエンド経由への切替）・Phase 6（テスト・品質）完了（トラッキング: #19）。SPA の OIDC PKCE ログイン（#60）と、モック IdP による認証 E2E・CSP（#50）も追加済みです。
