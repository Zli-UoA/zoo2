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

ビルドはイベントに記録された PR の head SHA を checkout し、同じ SHA を
Cloudflare のデプロイ情報に記録します。base と合成した merge commit は使いません。
PR ごとのデプロイは同時に1件だけ実行し、実行中・待機中のデプロイを新しい実行で
キャンセルしません（最大100件まで待機）。公開直前に GitHub API で PR の最新 head と
ビルド対象 SHA を照合し、一致しない実行や閉じた PR は公開をスキップします。
照合後に新しい push が来た場合は、進行中の公開が終わってから後続の実行が
照合・公開します。古い実行を後から再実行しても、公開済みの新しい内容を巻き戻しません。

## 初回設定

1. Cloudflare Pages に **Direct Upload** のプロジェクトを作成します。
   例としてプロジェクト名を `zoo2-storybook`、production branch を `main` にします。
   CLI なら `npx wrangler@4 pages project create zoo2-storybook --production-branch=main`
   で作成できます。Git 連携による自動ビルドは設定しません。
2. Cloudflare API Token を作成します。権限は対象アカウントの
   **Account / Cloudflare Pages / Edit** に限定します。この権限は、指定した
   プロジェクトだけでなく対象アカウント内の他の Pages プロジェクトにも及びます。
   他の Pages プロジェクトへの影響を分離するには、専用アカウントを使ってください。
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
- ダウンロードした生成物の直下から `_worker.js`（ディレクトリの場合も含む）、
  `_headers`、`_redirects`、`_routes.json` を除去し、PR が指定した Pages Functions
  やヘッダー・ルーティング設定を有効にしません。
- このプレビュー専用の Pages プロジェクトには、production・preview ともに
  Secrets・環境変数や KV / R2 / D1 などのバインディングを設定しないでください。
  公開するブラウザ用 JavaScript は PR の作成者が制御できるため、本番アプリと
  Cookie を共有するドメインも使わないでください。
- 同じリポジトリに書き込める人を、Cloudflare の Secrets を扱える人として
  信頼する構成です。`pull_request` は PR 側のワークフロー定義で実行されるため、
  ジョブの分離や生成物の処理では、ワークフロー自体の変更による Secrets 参照を
  防げません。この前提を変える場合は、信頼するブランチのワークフローでの
  デプロイと、保護した Environment での承認などを別途設計してください。
- Storybook プレビューは `automerge-gate` の集約対象から除外しています。
  プレビューの失敗や待機は自動マージを止めません。既存のフロントエンド CI にある
  Storybook のテスト・ビルドは引き続き集約対象です。
- このワークフローでは `main` の production デプロイは作成しません。
  Pages の production branch を `pr-<PR番号>` に設定しないでください。
- プレビューは公開 URL です。Story や fixture には公開可能なデータを使ってください。
- PR を閉じても Pages のデプロイと GitHub Environment は残ります。
  不要になったプレビューの削除手順は後述します。

## 再実行時に生成物が見つからない場合

ビルド生成物の保存期間は7日です。`Artifact not found for name: storybook-preview`
で失敗した場合、保存期限が切れているか、生成物が削除されています。
PR の現在の head SHA と一致する最新実行を選び、Actions の **Re-run all jobs**
でビルドから再実行するか、PR に push して
生成物を作り直してください。**Re-run failed jobs** だけでは成功済みの
ビルドが再実行されないため、生成物は復元されません。
古い SHA の実行は、生成物を作り直しても公開直前の照合でスキップされます。
この照合を導入する前のワークフロー実行は再実行しないでください。再実行は当時の
ワークフロー定義を使うため、今回追加した照合や直列化は適用されません。

## PR を閉じた後のプレビュー削除

Pages の画面ではブランチの最新デプロイを削除できない場合があります。
固定 URL の alias を持つ preview は、[Cloudflare の削除 API](https://developers.cloudflare.com/api/resources/pages/subresources/projects/subresources/deployments/methods/delete/)
の `force=true` を指定して削除します。対象 PR が閉じており、進行中・待機中の
デプロイがないことを確認してから、対象プロジェクトのデプロイ一覧で
`pr-<PR番号>` の preview の deployment ID を特定してください。

`DELETE /accounts/{account_id}/pages/projects/{project_name}/deployments/{deployment_id}?force=true`
に Pages Edit 権限の API Token で認証してリクエストします。対象 PR に属する
preview だけを削除し、production や他の PR のデプロイは削除しないでください。
削除後は固定 URL と各デプロイ URL が Storybook を返さないことを確認し、GitHub の
Settings → Environments から対応する `storybook-pr-<PR番号>` も削除します。
この手順は API の仕様に基づきます。実際のプロジェクトでの削除は未検証です。
