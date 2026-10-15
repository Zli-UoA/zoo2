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
pnpm build            # 本番ビルド
pnpm preview          # ビルドのプレビュー
pnpm check            # format / lint / typecheck
pnpm lint             # lint のみ
pnpm test             # vitest (Vite+ 内蔵)
pnpm generate-routes  # ルートツリーを再生成
```

ルートの Makefile からも実行できます: `make frontend-dev` / `frontend-build` / `frontend-check`。

開発サーバは Electric（ポート 3000）と衝突しないよう **5173** で起動します。

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

- `src/lib/config.ts` … `resolveConfig()` / `buildShapeUrl()`。`VITE_SHAPE_MODE` が `direct` なら Electric（`VITE_ELECTRIC_SHAPE_URL`）、`proxy` ならバックエンドの Shape プロキシ（`VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`）を参照します。未設定時は開発で `direct`、それ以外は `proxy`。不正な値は例外にします。**`direct` は Electric を認可なしで直接叩く開発専用**で、プライベートチャンネルのフィルタも効きません。プロキシ経由の検証や本番では `proxy` を使ってください。
- `src/lib/auth.ts` … `getToken()` のみのアダプタ。`setTokenProvider()` で IdP を差し替えられます（既定は未認証で `null`）。`buildAuthHeaders()`（`api/client.ts`）を使うと REST 側の fetch に同じ `Bearer` / `X-Dev-User` を渡せます。
- `src/lib/api/client.ts` … REST クライアント。`Bearer`（`getToken()`）と `X-Dev-User`（`VITE_DEV_USER_ID`、開発時のみ）を注入し、書き込み系の `{ data, txid }` エンベロープを取り出します（`unwrapEnvelope()`）。書き込みは Phase 3 以降で、Phase 2 の読み取りは Shape 購読のみを使います。
- `src/lib/electric/` … 行型とスキーマ定義。integer は文字列で届く前提で `coerceElectricRow()` が `Number()` 変換します（`int8` / `numeric` は精度のため文字列のまま）。timestamp は Shape が `2026-10-15 14:32:44.183`（空白区切り）、REST が `T` 区切りの文字列で、どちらも UTC の naive 値です。`normalizeUtcTimestamp()` / `parseUtcTimestamp()` で UTC の ISO 文字列・`Date` に正規化できます。

環境変数は `frontend/.env.example` を参照してください。

## リアルタイム購読（Phase 2）

`/realtime` に確認用の最小画面があります。`make db-up && make migrate` の後、開発サーバ（既定 `direct`）で開くと、チャンネル・メッセージ・投稿者名がリアルタイム表示されます。

- `@tanstack/electric-db-collection` で Electric Shape を購読し、`@tanstack/react-db` の `useLiveQuery` で表示します。書き込みと楽観更新は Phase 3、本格 UI は Phase 4 です。
- `src/lib/electric/collections.ts` … `channels` / `users` / `channel_user` / `messages` のコレクション。`messages` は `channel_id` 単位で生成してキャッシュします。ShapeStream はブラウザ API に依存するため、モジュール直下では作らず client コンポーネント / フックから `getCollections()` を呼びます（SSR では購読しません）。
- `src/lib/electric/query.ts` … `buildShapeUrl()` に渡す Shape クエリ。`direct` では Electric の subset プロトコル（`where` + `params[1]`）、`proxy` では `channel_id` を渡します（`table` は direct では `buildShapeUrl()`、proxy ではバックエンドが付与）。
- `src/lib/electric/headers.ts` … ShapeStream 用 fetch。REST と同じ `Bearer` / `X-Dev-User` を「値があるときだけ」付与します。常時付与すると開発時の Electric 直結で `authorization` が CORS の preflight に弾かれるためです。
- `src/lib/electric/messageFeed.ts` … `messages ⋈ users` の live query。投稿者名は非正規化せず `users` Shape を結合して表示し、`deleted_at` をクエリ側で除外します。
- `src/hooks/useRealtime.ts` … 上記コレクションを購読するフック（チャンネル一覧・メッセージ・メンバー・ユーザー）。

## 現状

Phase 0（足場）・Phase 1（型・データアクセス層）・Phase 2（読み取り・live query）完了。書き込みと楽観更新は Phase 3、shadcn の本格 UI は Phase 4 です（トラッキング: #19）。
