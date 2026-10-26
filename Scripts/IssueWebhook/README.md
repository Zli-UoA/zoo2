# IssueのWebhook受信

`Zli-UoA/zoo2-ios` のIssue作成・編集・再オープン・コメントを受信し、既存のCodex開発チャットを
起動します。非公開リポジトリに対応し、追加のOpenAI APIキーや公開Webhook中継は使いません。

## 構成

[GitHub公式の開発用転送](https://docs.github.com/en/webhooks/testing-and-troubleshooting-webhooks/using-the-github-cli-to-forward-webhooks-for-testing)
と同じ `cli` Webhookと認証付きWebSocketを使います。
[公式CLIの転送処理](https://github.com/cli/gh-webhook/blob/v0.2.0/webhook/forward.go)が扱う生の本文と
ヘッダーを受け取り、HMAC-SHA256を検証します。本文をJSONへ再変換してから照合しません。
署名が一致しない通知は破棄し、照合を省略する設定は用意しません。

コード・設定の変更対象はzoo2-iosだけです。本文の要求を権限付与として扱いません。
GitHubから通知を起こした `sender` の現在の書き込み権限を照会してからCodexへ通知します。
`author_association` やIssue原著者の情報だけで許可しません。PRとBotからは起動しません。
権限がない投稿も、既存の毎時確認では検討対象にできます。

## ローカル設定

Node.js 22.13以降、ログイン済みGitHub CLI、対象チャットへアクセスできるCodex CLIが必要です。
`npm ci --ignore-scripts --prefix Scripts/IssueWebhook` で依存を導入します。
実運用ではレビュー済みコードを `.autodev/receiver/` へ配置し、worktreeの削除やブランチ変更で
受信コードが変わらないようにします。

Git管理対象外の `.autodev/webhook/config.json` に次の項目を保存します。

```json
{
  "repositoryPath": "/絶対パス/zoo2-ios",
  "repositoryID": 123456789,
  "hookID": 123456,
  "websocketURL": "GitHubから返されたwss接続先",
  "secretFile": "/絶対パス/zoo2-ios/.autodev/webhook/secret",
  "threadID": "既存コントローラーチャットのUUID",
  "codexPath": "/絶対パス/codex",
  "ghPath": "/絶対パス/gh"
}
```

署名用の秘密値は暗号学的乱数から生成し、GitHubのWebhook設定とsecretファイルに同じ値を
登録します。ディレクトリ700、ファイル600の権限にし、設定・secret・実行記録はコミットしません。
GitHubトークンはログイン済みCLIからメモリーへ取得し、GitHubの認証付き接続だけに渡します。
本文・トークン・署名用秘密値・WebSocket接続先はログへ出しません。

Webhookは `name=cli`、JSON形式、TLS検証有効、`issues` と `issue_comment` のみに設定します。
既存のCLI Webhookがある場合は上書き・削除せず、所有者と接続先を確認してください。
この転送は1リポジトリにつき同時に1接続だけが使う機能です。

## 起動と常駐化

`node Scripts/IssueWebhook/run.mjs /絶対パス/config.json` で起動できます。
SQLite排他ロックを取得してから通知記録を読み、二重更新を防ぎます。異常終了時にもOSがロックを
解放します。WebSocketの切断時は15秒後に再接続します。

常駐化は初回のユーザー承認後に、launchdのユーザー用ジョブとして登録します。
実行ファイル・設定・作業ディレクトリを絶対パスで指定し、出力は `.autodev/webhook/` に保存します。
Macの電源断・スリープ中は受信できません。GitHub公式の転送機能自体も開発・検証向けであり、
本番サービスのWebhook基盤としての配信保証はありません。毎時確認を補完として継続します。

## 確認方法

1. `npm test --prefix Scripts/IssueWebhook` で署名・対象制限・権限・重複・再試行を検証します。
2. GitHubのpingが受信記録に現れることを確認します。pingではAIを起動しません。
3. 対象Issueへ日本語の検証コメントを投稿します。
4. GitHub配信、受信記録の `queued`、対象チャットの実際の処理を順に照合します。
5. `.autodev/webhook/acknowledgements/<本文ダイジェスト>.json` が配信内容と対応することを確認します。
6. 同じ配信を再送し、受信処理への再到達と重複起動の有無を確認します。GitHub側で再送が失敗した場合は、重複防止の成功と混同せず、新規通知の受信とローカルの再投入検証を分けて記録します。

応答不明の際などは起動要求が再送されることがあります。同じ本文ダイジェストの確認記録があれば、
重複実装しません。権限照会または起動が3回失敗した通知は `failed` のまま保持します。
原因を確認せず記録を消したり、成功扱いにしたりしません。
