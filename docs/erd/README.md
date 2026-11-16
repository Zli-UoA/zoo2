# Liam ERD

`infra/db/schema.sql` から検索・拡大縮小できるER図を生成し、Cloudflare Pagesの `zoo2-erd` へ公開します。データベースへの接続やデータの読み出しは行いません。

公開先（初回デプロイ後）: https://zoo2-erd.pages.dev

## ローカルでの生成

開発シェル（`nix develop`）のNode.js 22とnpm、およびDockerを使用します。PostgreSQL 16の一時コンテナへスキーマを適用し、`pg_dump --schema-only` の結果をLiamへ渡します。参照先の列を省略した複合外部キーも、この段階で解決します。コンテナと一時SQLは終了時に削除します。

```sh
cd docs/erd
npm ci
npm run build
```

`dist/` が配信ファイルです。Liam CLIは `package-lock.json` で依存を固定しています。既存のMermaid図とその更新チェックは継続します。

## 自動更新

`cd-db-erd.yaml` はPRでビルドを検証し、mainのスキーマ・生成設定の更新でビルドとデプロイを実行します。`workflow_dispatch` によるmainからの再配信もできます。PRでは配信せず、生成物をActionsのartifactに保存します。ビルドとSecretを使う配信は別ジョブです。

## 初回の配備順序

1. [zoo2-infra](https://github.com/Zli-UoA/zoo2-infra) の `terraform/cloudflare/pages.tf` を先にマージし、OpenTofuのapply成功を確認します。プロジェクトの作成・削除はこのstateで管理します。
2. zoo2-infraのActions Secret `CLOUDFLARE_API_TOKEN` に、対象アカウントの **Cloudflare Pages: Edit** を追加します（既存のR2等の権限も維持）。zoo2側はStorybookと同じ `CLOUDFLARE_API_TOKEN`・`CLOUDFLARE_ACCOUNT_ID` を使います。トークンにはPagesの編集権限が必要です。未設定の場合、配信は明示的に失敗します。
3. zoo2の変更をmainへマージするか、mainで `DB ER diagram` を手動実行します。
4. 公開URLでテーブル検索・外部キーの線・拡大縮小を確認します。

スキーマ定義は公開静的ファイルに含まれます。DB接続情報や行データを入力に追加しないでください。
