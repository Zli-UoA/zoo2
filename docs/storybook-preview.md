# Storybook の PR プレビュー

PR の作成・更新（push）・再オープン時に、`Storybook Preview` ワークフローが
Storybook をビルドして Cloudflare Pages へデプロイします。draft PR や、
フロントエンド以外だけを変更した PR も対象です。同じリポジトリ内の PR のみ
公開し、Secrets を使えない fork からの PR はスキップします。

PR のデプロイ欄に表示される `storybook-pr-<PR番号>` のリンク、または
Actions の実行サマリーの `PR preview` から開けます。
`https://pr-<PR番号>.<プロジェクト名>.pages.dev` は PR ごとに固定で、
更新時に最新の成功したデプロイへ切り替わります。`This deployment` は
その実行で公開した内容の固定 URL です。ビルド・デプロイに失敗した場合は、
前回成功時のプレビューが残るため、Actions の結果も確認してください。

## 初回設定

1. Cloudflare Pages に **Direct Upload** のプロジェクトを作成します。
   例としてプロジェクト名を `zoo2-storybook`、production branch を `main` にします。
   CLI なら `npx wrangler@4 pages project create zoo2-storybook --production-branch=main`
   で作成できます。Git 連携による自動ビルドは設定しません。
2. Cloudflare API Token を作成します。権限は対象アカウントの
   **Account / Cloudflare Pages / Edit** に限定します。
3. GitHub リポジトリの Settings → Secrets and variables → Actions に次を登録します。

| 種別 | 名前 | 値 |
| --- | --- | --- |
| Secret | `CLOUDFLARE_API_TOKEN` | 作成した API Token |
| Secret | `CLOUDFLARE_ACCOUNT_ID` | Pages プロジェクトを置いたアカウント ID |
| Variable | `CLOUDFLARE_PAGES_PROJECT_NAME` | Pages のプロジェクト名（例: `zoo2-storybook`） |

これらは PR ごとの GitHub Environment ではなく、リポジトリに登録します。
未設定ならデプロイジョブが設定名を表示して失敗します。
ワークフローをデフォルトブランチへ反映した後、同じリポジトリの PR を
作成するか既存 PR に push し、公開 URL で Storybook を確認してください。

## 配信と運用

- `pnpm build-storybook` の生成物だけを配信します。Storybook のモック API を
  使用するため、バックエンドや本番ログインの設定は不要です。
- ビルドは既存の runner 選択に従います。デプロイは毎回新しい GitHub-hosted
  runner で行い、Cloudflare の認証情報を持つジョブでは PR のコードを checkout
  したり、PR の依存関係・ビルドスクリプトを実行したりしません。
- このワークフローでは `main` の production デプロイは作成しません。
  Pages の production branch を `pr-<PR番号>` に設定しないでください。
- プレビューは公開 URL です。Story や fixture には公開可能なデータを使ってください。
- PR を閉じても Pages のデプロイと GitHub Environment は残ります。
  不要になったプレビューは Cloudflare Pages と GitHub の画面で削除してください。
