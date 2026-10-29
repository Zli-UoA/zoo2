# 配備状況のバッジ

READMEのバッジは、GitHubに記録されたFluxの結果を専用の `deployment-badges` ブランチへ写したものです。アプリの死活監視ではありません。

| 表示 | 意味 |
| --- | --- |
| Flux main: success | 最新mainの対象リソースすべてに成功の通知があります |
| Flux main: pending | 最新mainの通知がまだないか、一部の結果を待っています |
| Flux main: failure | 最新mainの対象リソースに失敗の通知があります |
| Flux main: unknown | GitHub APIから状態を取得できませんでした |
| production | GitHubの最新production Deploymentの結果とコミットです。最新mainと一致するとは限りません |
| checked UTC | バッジを生成した確認時刻です。Fluxイベントの発生時刻は詳細ページに表示します |

AuthZとzoo2のmain判定には、migrationとアプリの両方の `flux/*` ステータスを使います。通知workflowが成功しただけでは反映成功にしません。production Deploymentが未登録ならproductionはunknownです。infraのmain判定には `.github/deployment-badges.json` に列挙したKustomizationを使い、未通知の対象があればpendingにします。対象を追加・廃止したときは、この一覧も更新してください。

READMEのバッジをクリックすると、対象コミット、リソースごとの結果・イベント時刻、配備履歴、更新workflowを確認できます。バッジはGitHub内の画像を参照するため、非公開リポジトリの閲覧にはアクセス権が必要です。外部サービスへトークンや状態を送信しません。

## 更新と権限

`Deployment badges` workflowはmainへのpush、Fluxのcommit status通知、`Flux production status` workflowの完了、毎時17分の定期実行、手動実行で更新します。GITHUB_TOKENで作成されたcommit statusからは別workflowが起動しないため、AuthZ・zoo2では通知workflowの完了も更新の契機にしています。workflow_runから実行するコードはmainに固定しています。

更新処理はGitHub APIで最新mainのSHAを確定し、そのSHAのステータスを取得します。API取得に失敗した場合はunknownを生成します。API障害やrunner待ちで生成結果を保存できない場合は前回の表示が残るため、checked UTCも確認してください。定期実行には遅延があり、更新間隔の保証はありません。

生成するのはSVG、`status.json`、詳細用READMEだけです。初回実行でコード履歴から独立した専用ブランチを作り、以降は強制pushせず更新します。mainのファイルは書き換えません。専用ブランチにはworkflowを置かず、生成コミットには `[skip ci]` を付けます。

追加のSecretは不要です。更新jobには `contents: write`、`statuses: read`、`deployments: read` を付けます。ブランチの作成・更新がrulesetなどで拒否された場合は、workflowの失敗を確認してください。

マージ後の初回実行までは専用ブランチが存在せず、READMEの画像は表示されません。zoo2では通知受信側のPR #177がmainに入るまで、Fluxの結果が記録されずpendingが続きます。

## 確認と停止

ローカルの検証は次のコマンドで行います。

```sh
node --test .github/scripts/deployment-badges.test.mjs
```

マージ後はworkflowの成功、専用ブランチの生成、ログイン済みGitHubでのREADME画像表示を確認してください。main更新直後のpending、成功・失敗の通知後の変化、本番コミットと時刻の一致も確認します。

更新を止めるときはworkflowを無効化し、READMEのバッジを外してください。専用ブランチを削除する場合も先にworkflowを停止します。
