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
- `src/lib` … ユーティリティとデータアクセス層（`config.ts` / `auth.ts` / `api/` / `electric/`）
- `src/styles.css` … Tailwind v4 と shadcn テーマ
- `components.json` … shadcn 設定（`pnpm dlx shadcn@latest add <name>` で追加）

`src/routeTree.gen.ts` は生成物です。追跡はしますがフォーマッタ対象からは除外しています（`.prettierignore`）。

## データアクセス層（Phase 1）

Electric の Shape 購読と REST 書き込みを分離し、接続先を設定だけで切り替えられるようにしています。UI とコレクション購読は Phase 2 以降です。

- `src/lib/config.ts` … `resolveConfig()` / `buildShapeUrl()`。`VITE_SHAPE_MODE` が `direct` なら Electric（`VITE_ELECTRIC_SHAPE_URL`）、`proxy` ならバックエンドの Shape プロキシ（`VITE_API_BASE_URL` + `VITE_SHAPE_PROXY_PATH`）を参照します。`direct` では Electric に必要な `table` パラメータを付与します。
- `src/lib/auth.ts` … `getToken()` のみのアダプタ。`setTokenProvider()` で IdP を差し替えられます（既定は未認証で `null`）。
- `src/lib/api/client.ts` … REST クライアント。`Bearer`（`getToken()`）と `X-Dev-User`（`VITE_DEV_USER_ID`）を注入し、書き込み系の `{ data, txid }` エンベロープを取り出します（`unwrapEnvelope()`）。
- `src/lib/electric/` … 行型とスキーマ定義。integer は文字列で届く前提で `coerceElectricRow()` が `Number()` 変換し、timestamp は ISO 文字列のまま扱います。

環境変数は `frontend/.env.example` を参照してください。

## 現状

Phase 0（足場）と Phase 1（型・データアクセス層）完了。以降の実装は Issue を参照してください（トラッキング: #19）。
