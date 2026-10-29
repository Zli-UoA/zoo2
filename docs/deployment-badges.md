# 配備状況のバッジ

READMEのバッジは、GitHubに記録されたFluxの結果を共用GistのJSONへ保存し、Shields.ioのEndpointで画像にしたものです。アプリの死活監視ではありません。

| 表示 | 意味 |
| --- | --- |
| Flux main: success | 最新mainの対象すべてに成功の通知があります |
| Flux main: pending | 最新mainの通知がまだないか、一部の結果を待っています |
| Flux main: failure | 最新mainの対象に失敗の通知があります |
| Flux main: unknown | GitHub APIから状態を取得できませんでした |
| production | 最新production Deploymentの結果と7桁のコミットSHAです。最新mainと一致するとは限りません |
| checked UTC | 状態を取得した確認時刻です。初期状態ではnot checkedと表示します |

AuthZとzoo2はmigrationとアプリの両方の`flux/*`ステータスでmainを判定します。production Deploymentが未登録ならproductionはunknownです。infraは設定JSONに列挙したKustomizationを使い、未通知の対象があればpendingにします。対象を追加・廃止した場合は一覧も更新してください。通知workflowの成功だけでは、配備成功にしません。

バッジをクリックすると、mainのコミット一覧、productionの配備履歴、バッジの更新workflowへ進めます。リソースごとの結果・イベント時刻・完全なSHAは、更新jobのSummaryで確認できます。READMEの上部には説明文を追加しません。

## Gistと公開する情報

保存先は[kashu-02所有の共用Secret Gist](https://gist.github.com/kashu-02/4def5a543fcb732d3ed5aa9182568925)です。Secret GistもURLを知っていれば読めます。Gistへ保存するのはバッジの状態、7桁のSHA、確認時刻と、表示用のラベル・色だけです。完全なSHA、対象リソースの詳細、ログ、トークンは公開JSONへ含めません。

ファイル名は`<prefix>-<kind>.json`です。prefixは`authz`・`zoo2`・`infra`、kindは`main`・`production`・`updated`とし、infraにはproductionを作りません。合計8ファイルを使い、各リポジトリは自分のファイルだけを1回のPATCHで更新します。他リポジトリのファイルやGistの説明は上書きしません。

JSONはShields.ioのEndpoint形式です。raw URLにはGistのリビジョンを含めず、READMEではURL全体をエンコードしてEndpointのurlパラメーターに渡します。

```json
{"schemaVersion":1,"label":"Flux main","message":"success a1b2c3d","color":"#2ea043"}
```

`cacheSeconds=300`を指定しますが、Shields.ioとGitHubのキャッシュ、runner待ち、定期実行の遅延により表示が遅れることがあります。即時更新の保証はありません。確認時刻も見てください。

## 初期設定と権限

Gist IDとprefixは`.github/deployment-badges.json`で管理します。Secret Gistは8ファイルともunknown、確認時刻はnot checkedで初期化済みです。初期値は本番の状態を表しません。

自動更新には、Gist所有者kashu-02が発行したfine-grained PATを使います。ユーザー権限のGistsをRead and writeにし、リポジトリへの書き込み権限は付けません。この権限はGist単位には限定できないため、他のGistも更新できるトークンとして管理します。

PATは次の3リポジトリのActions Secretへ、同じ名前`BADGE_GIST_TOKEN`で登録します。以下のコマンドはトークンを対話入力します。値をコマンド引数、チャット、Git、ログに残さないでください。

```sh
gh secret set BADGE_GIST_TOKEN --repo Zli-UoA/AuthZ
gh secret set BADGE_GIST_TOKEN --repo Zli-UoA/zoo2
gh secret set BADGE_GIST_TOKEN --repo Zli-UoA/zoo2-infra
```

GitHubのステータス・Deploymentの読取には通常のGITHUB_TOKENを使い、Gist更新だけにBADGE_GIST_TOKENを使います。更新jobのリポジトリ権限はcontents: read、statuses: read、deployments: readです。PRのテストjobにはGist用トークンを渡しません。workflow_runの公開jobはmainのコードだけを実行します。

PAT交換時は新しいトークンを3リポジトリへ登録し、手動実行でGistの更新を確認してから旧トークンを失効させます。未登録・期限切れ・権限不足の場合はjobが失敗します。CLIのログイン用トークンを自動更新用Secretとして流用しません。

## 更新と障害時の動作

更新の契機はmainへのpush、Fluxのcommit status通知、Flux production status workflowの完了、毎時17分の定期実行、手動実行です。GITHUB_TOKENによるstatus登録から別workflowが起動しないため、AuthZ・zoo2では通知workflowの完了も使います。runner選択とCI_RUNNERの扱いは既存のCIと共通です。

最新mainのSHAを確定してから、そのSHAのステータスを取得します。取得に失敗した場合はunknownを保存します。Gist更新の429・5xx・通信エラーは合計3回まで試み、更新できなければjobを失敗させます。認証・権限エラーは再試行しません。更新できない間は前回の表示が残るため、確認時刻とActionsの失敗を確認してください。

Gitのブランチ・コミットは生成せず、自前のSVG生成も行いません。旧deployment-badgesブランチが存在しても、この変更では削除しません。

## 検証と停止

```sh
node --test .github/scripts/deployment-badges.test.mjs
gh workflow run deployment-badges.yaml --repo Zli-UoA/AuthZ
gh workflow run deployment-badges.yaml --repo Zli-UoA/zoo2
gh workflow run deployment-badges.yaml --repo Zli-UoA/zoo2-infra
```

マージ後はSecret登録、各workflowの成功、認証なしでのraw JSON取得、Shields.ioとREADMEの画像表示を確認します。状態・短縮SHA・確認時刻がGitHubの記録と一致すること、各リポジトリの更新が他のJSONを保持することも確認してください。

zoo2の通知受信側PR #177が未マージの間は、Fluxの結果が記録されずpendingが続きます。取り込み後の本番通知による変化は別途確認します。

停止時は更新workflowを無効化し、READMEのバッジを外します。Gistは3リポジトリ共用のため、1つのリポジトリの停止だけでGistを削除しないでください。
