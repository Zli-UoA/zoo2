# Frontend

Zoo2 のフロントエンド。TanStack Start（React 19）+ Vite+ + Tailwind CSS v4 + shadcn/ui（Base UI）。

## 必要環境

- Nix devShell（Node.js 22 と pnpm を供給）。リポジトリルートで `nix develop`。
- バックエンド / Electric を動かす場合はルートで `make db-up && make migrate`。

## セットアップ

```bash
cd frontend
pnpm install
```

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
pnpm generate-routes  # ルートツリーを再生成
```

ルートの Makefile からも実行できます: `make frontend-dev` / `frontend-build` / `frontend-check` / `frontend-test` / `frontend-e2e`。

開発サーバは Electric（ポート 3000）と衝突しないよう **5173** で起動します。

本番ビルドは TanStack Start の SPA モード（`spa: { enabled: true }`）を使い、プリレンダリングされた SPA シェル（`dist/client/index.html`）とクライアントアセットを `dist/client/` に出力します。コンテナでの静的配信と Coolify の設定は [インフラの README](../infra/README.md#ステージング) を参照してください。

## 構成

- `src/routes` … ファイルベースルーティング（TanStack Router）
- `src/components/ui` … shadcn/ui（Base UI）コンポーネント
- `src/hooks` … live query の購読フック（`useRealtime.ts`）
- `src/lib` … ユーティリティとデータアクセス層（`config.ts` / `auth.ts` / `api/` / `electric/`）
- `src/styles.css` … Tailwind v4 と shadcn テーマ
- `components.json` … shadcn 設定（`pnpm dlx shadcn@latest add <name>` で追加）

`src/routeTree.gen.ts` は生成物です。追跡はしますがフォーマッタ対象からは除外しています（`.prettierignore`）。

## データアクセス層（Phase 1）

Electric の Shape 購読と REST 書き込みを分離し、接続先を設定だけで切り替えられるようにしています。

- `src/lib/config.ts` … `resolveConfig()` / `buildShapeUrl()`。`VITE_SHAPE_MODE` が `direct` なら Electric（`VITE_ELECTRIC_SHAPE_URL`）、`proxy` ならバックエンドの Shape プロキシ（`VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`）を参照します。**既定は `proxy`**（開発・本番とも）。不正な値は例外にします。開発で `proxy` を使うときは現在ユーザー（`X-Dev-User`）が必要なため `VITE_DEV_USER_ID` を必須にします（未設定は起動時エラー）。**`direct` は Electric を認可なしで直接叩く開発専用**で、プライベートチャンネルのフィルタも効きません。
- `src/lib/auth.ts` … `getToken()` のみのアダプタ。`setTokenProvider()` で IdP を差し替えられます（既定は未認証で `null`）。`buildAuthHeaders()`（`api/client.ts`）を使うと REST 側の fetch に同じ `Bearer` / `X-Dev-User` を渡せます。
- `src/lib/api/client.ts` … REST クライアント。`Bearer`（`getToken()`）と `X-Dev-User`（`VITE_DEV_USER_ID`、開発時のみ）を注入し、書き込み系の `{ data, txid }` エンベロープを取り出します（`unwrapEnvelope()`）。書き込みは Phase 3 以降で、Phase 2 の読み取りは Shape 購読のみを使います。
- `src/lib/electric/` … 行型とスキーマ定義。integer は文字列で届く前提で `coerceElectricRow()` が `Number()` 変換します（`int8` / `numeric` は精度のため文字列のまま）。timestamp は Shape が `2026-10-15 14:32:44.183`（空白区切り）、REST が `T` 区切りの文字列で、どちらも UTC の naive 値です。`normalizeUtcTimestamp()` / `parseUtcTimestamp()` で UTC の ISO 文字列・`Date` に正規化できます。

環境変数は `frontend/.env.example` を参照してください。

## リアルタイム購読（Phase 2）

`make db-up && make migrate` の後、開発サーバ（既定 `proxy`。バックエンドと `VITE_DEV_USER_ID` を用意。下記 Phase 5）で開くと、チャンネル・メッセージ・投稿者名がリアルタイム表示されます。表示は Phase 4 の画面（`/` と `/channels/$channelId`）で行います。

- `@tanstack/electric-db-collection` で Electric Shape を購読し、`@tanstack/react-db` の `useLiveQuery` で表示します。書き込みと楽観更新は Phase 3、画面は Phase 4（いずれも下記）です。
- `src/lib/electric/collections.ts` … `channels` / `users` / `channel_user` / `messages` のコレクション。`messages` は `channel_id` 単位で生成してキャッシュします。ShapeStream はブラウザ API に依存するため、モジュール直下では作らず client コンポーネント / フックから `getCollections()` を呼びます（SSR では購読しません）。
- `src/lib/electric/query.ts` … `buildShapeUrl()` に渡す Shape クエリ。`direct` では Electric の subset プロトコル（`where` + `params[1]`）、`proxy` では `channel_id` を渡します（`table` は direct では `buildShapeUrl()`、proxy ではバックエンドが付与）。
- `src/lib/electric/headers.ts` … ShapeStream 用 fetch。REST と同じ `Bearer` / `X-Dev-User` を「値があるときだけ」付与します。常時付与すると開発時の Electric 直結で `authorization` が CORS の preflight に弾かれるためです。
- `src/lib/electric/messageFeed.ts` … `messages ⋈ users` の live query。投稿者名は非正規化せず `users` Shape を結合して表示し、`deleted_at` をクエリ側で除外します。
- `src/hooks/useRealtime.ts` … 上記コレクションを購読するフック（チャンネル一覧・メッセージ・メンバー・ユーザー）。

## 書き込みと楽観更新（Phase 3）

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

## 画面（Phase 4）

shadcn（Base UI）のテーマのみで最小の画面を組んでいます。細かい Tailwind の作り込みは行いません。

- ルート `/` … チャンネル一覧・作成・参加/退出・メンバー表示（`src/routes/index.tsx` → `src/components/ChannelDirectory.tsx`）。各チャンネルは `Link` でメッセージ画面へ遷移します。
- ルート `/channels/$channelId` … メッセージ一覧・送信・編集・論理削除・投稿者表示（`src/routes/channels/$channelId.tsx` → `src/components/MessagePane.tsx`）。
- ルート `/realtime` … 検証 UI は上記の実ルートへ移行しました。ここは接続モード（Shape / txid / 開発ユーザー）を表示する最小画面です（`src/components/SyncStatus.tsx`）。
- `src/hooks/useWriteRunner.ts` … `collection.insert()` などの楽観更新トランザクションを `when("settled")` まで待ち、失敗を UI のエラーメッセージにまとめます。
- 編集・削除は自分のメッセージにのみ表示します（バックエンドは投稿者のみ許可するため）。開発で `proxy` モードを使うときは `VITE_DEV_USER_ID` が必須です（未設定は起動時エラー）。`frontend/.env.example` の例にならい設定してください。
- 非公開チャンネルは参加者だけに説明・メンバー・メッセージを表示します。`proxy` の Shape はバックエンドが認可で絞るため未参加者には届きませんが、UI 側でも `isMember` で隠します。`direct` は認可せず全件配信するため UI 側のガードだけが頼りです。`/channels/$channelId` を URL 直開きしても未参加ならメッセージは出しません。説明の表示はメンバーパネル側で行います。
- メンバー追加フォームはバックエンドの `add_channel_user` と同じ条件（作成者は常に、非公開は作成者のみ、公開は作成者またはメンバー）でのみ表示します。
- `ChannelMemberPanel` はチャンネルごとに置きますが、`channel_user` の Shape は `getCollections()` が 1 本だけ作り、パネルはそのコレクションを購読します。`users` は全パネルで同一クエリのため TanStack DB 内部でプール共有されます。同じ channel の `useChannelMembers`（画面とパネル）もプールされ共有されるため、購読はチャンネル数に対して増えません。
- 既存のガードを踏襲します。重複メンバーの追加は UI 側で弾き、現在のユーザー以外の退出は書き込みハンドラが throw してロールバックします。メンバー読み込み中は重複判定が効かないため参加/追加ボタンを無効化します。

## バックエンド経由への切替（Phase 5）

Shape の接続先をバックエンドの Shape プロキシ（`/api/shapes/*`）に切り替え、`awaitTxId` を既定で有効にしました（`VITE_TXID_MODE=awaitTxId`）。既定の `VITE_SHAPE_MODE` は `proxy`（開発・本番とも）です。

### セットアップ

```bash
make db-up && make migrate        # PostgreSQL + Electric
make backend-dev                  # DEV_AUTH_ENABLED=true cargo run (http://localhost:8000)
cp frontend/.env.example frontend/.env
cd frontend && pnpm dev           # http://localhost:5173
```

`frontend/.env` では `VITE_SHAPE_MODE=proxy` と `VITE_DEV_USER_ID`（現在ユーザー。`X-Dev-User` として送信。DB の `users` に存在する id）を設定します。**開発で `proxy` のときは `VITE_DEV_USER_ID` が必須**で、無いと起動時エラーになります（`.env` のコピー忘れに注意）。バックエンドの既定ユーザーは `DEV_DEFAULT_USER_ID`（既定 `00000000-0000-0000-0000-000000000001`）で、`X-Dev-User` が無いときはこのユーザーとして扱われます。

### 経路とポイント

- 読み取り: `proxy` は `VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`（既定 `/api/shapes`）を購読します。バックエンドが `table` / `where` を確定し、サーバ側で `ELECTRIC_SECRET` を付けて Electric へ転送します。`messages` / `channel_users` はクエリで `channel_id` を渡します。
- 認証: REST と Shape の両方で同じ `X-Dev-User`（と `Authorization`）を受け付けます。バックエンドは `DEV_AUTH_ENABLED=true` のときだけこのヘッダを受け付け、それ以外は 403 です。CORS はバックエンドが `CORS_ALLOWED_ORIGINS` の許可オリジンに限定し（`ENVIRONMENT=production` 以外で未設定なら `http://localhost:5173`, `http://127.0.0.1:5173`、本番で未設定なら空）、メソッドとヘッダも既知のものだけを許可しつつ `electric-*` を公開します。`direct` は Electric の CORS が `X-Dev-User` / `Authorization` を許可しないため認証ヘッダを付けません（`collections.ts` の `shapeAuthSource`）。
- 認可: バックエンドの channels Shape は `is_private = false OR id IN (SELECT channel_id FROM channel_user WHERE user_id = ...)` でメンバーシップに絞ります。未参加の非公開チャンネルは Shape・メッセージのどちらも届きません（存在隠蔽のため 404）。参加/退出で channels 側の行が move-in / move-out します。
- `channel_users` のサブクエリ経路（`electric#3143`）: Electric 1.8.1 を `ELECTRIC_FEATURE_FLAGS=allow_subqueries` で動かしており、`WHERE ... IN (SELECT ... WHERE user_id = $1)` の形がそのまま通ることを実測しました（`params[1]` により move-in / move-out も配信）。`$1` をリテラルへ埋め込む回避策は不要でした。
- 書き込み: すべて REST（`{ data, txid }`）で行い、`txid` を `awaitTxId` で Electric の配信と突き合わせます。失敗時は楽観状態がロールバックします。

## テスト・品質（Phase 6）

主要なデータ変換と書き込みフローを vitest で担保し、`vp check`（format / lint / typecheck）とテストを CI で回します。

- ユニットテスト（vitest / `src/**/*.test.ts`）… 数値・真偽値の文字列変換（`schema.test.ts`）、timestamp 正規化（`timestamps.test.ts`）、設定解決（`config.test.ts`）、REST アダプタ `{ data, txid }` の注入と剥がし・エラー化（`api/client.test.ts`）、`txid` ハンドシェイク（`txid.test.ts` / `writes.test.ts`）、Shape の URL / ヘッダ / live query（`electric/*.test.ts`）。
- CI … `.github/workflows/ci-frontend.yaml`。`frontend/**` の変更（`main` への push / PR）で `pnpm check`・`pnpm build`・`pnpm test` を実行します（バックエンドの CI とは別ワークフロー）。
- スモーク（Playwright / `e2e/smoke.spec.ts`）… 起動 → チャンネル作成 → メッセージ送信 → Electric 反映までを実ブラウザで確認します。`pnpm test` には含めず `pnpm test:e2e` で明示実行します。前提として DB + Electric + バックエンドが必要です。

```bash
make db-up && make migrate        # PostgreSQL + Electric
make backend-dev                  # DEV_AUTH_ENABLED=true (http://localhost:8000)
cd frontend && pnpm test:e2e      # 起動 → 送信 → 反映
```

`pnpm test:e2e` はフロントエンド（proxy モード）だけを起動し、バックエンドは既存のものを再利用します。現在ユーザー（`VITE_DEV_USER_ID`、既定 `...0001`）が未作成なら `POST /api/users` でスモーク用に作成します。

## 現状

Phase 0（足場）・Phase 1（型・データアクセス層）・Phase 2（読み取り・live query）・Phase 3（書き込み・楽観更新）・Phase 4（UI: shadcn テーマのみ）・Phase 5（バックエンド経由への切替）・Phase 6（テスト・品質）完了（トラッキング: #19）。
