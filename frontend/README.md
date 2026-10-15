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
pnpm generate-routes  # ルートツリーを再生成
```

ルートの Makefile からも実行できます: `make frontend-dev` / `frontend-build` / `frontend-check`。

開発サーバは Electric（ポート 3000）と衝突しないよう **5173** で起動します。

## 構成

- `src/routes` … ファイルベースルーティング（TanStack Router）
- `src/components/ui` … shadcn/ui（Base UI）コンポーネント
- `src/lib` … ユーティリティ
- `src/styles.css` … Tailwind v4 と shadcn テーマ
- `components.json` … shadcn 設定（`pnpm dlx shadcn@latest add <name>` で追加）

`src/routeTree.gen.ts` は生成物です。追跡はしますがフォーマッタ対象からは除外しています（`.prettierignore`）。

## 現状

Phase 0（足場）完了。以降の実装は Issue を参照してください（トラッキング: #19）。
