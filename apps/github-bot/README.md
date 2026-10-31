# GitHub 通知 Bot for Zoo

GitHub 上の変更を Zoo に反映する、独立した Bot アプリです。Zoo の App API・TypeScript SDK・宣言型 UI を使います。Zoo 本体に GitHub 専用 Webhook を追加しません。

- Issue / PR ごとに親メッセージを作成し、タイトル・本文・ラベル・担当者・レビュアー・レビュー結果・Checks・マージ状態を編集します。
- GitHub のコメント・レビュー・状態変更は同じ親のスレッドへ返信します。コメントやレビューの編集は既存の返信を更新し、コメント削除は該当する返信を削除します。
- Actions とデプロイも、同じ実行／デプロイの進行状況を親の編集と返信で追跡します。
- Zoo から GitHub の Issue、PR、コメント、レビュー、Actions、デプロイを更新する機能はありません。GitHub App の権限はすべて読み取り専用です。

## 準備

Node.js 24 以上が必要です。実行時の npm 依存はありません。SQLite と暗号処理には Node.js の標準モジュールを使います。

### 1. GitHub App を作る

[GitHub App の作成手順](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)に従い、[github-app-manifest.example.json](github-app-manifest.example.json) を参考に設定します。App 名と公開 URL は自分の環境に変更してください。

| 設定                       | 値                                                                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Webhook URL                | `https://BOT_HOST/github/webhook`                                                                                              |
| Webhook Secret             | 十分長いランダムな値                                                                                                           |
| Repository permissions     | Metadata / Contents / Issues / Pull requests / Checks / Commit statuses / Actions / Deployments / Discussions は **Read-only** |
| Webhook events             | サンプル manifest の `default_events`                                                                                          |
| User authorization / OAuth | 不要                                                                                                                           |

対象リポジトリへインストールし、App ID、Installation ID、生成した RSA 秘密鍵を控えます。Installation の作成・停止・削除とリポジトリ追加／削除のイベントも受け取り、権限を失ったリポジトリの処理を止めます。

### 2. Zoo App を作る

Zoo の「App・連携」から GitHub 通知用 App を作成し、[manifest.example.json](manifest.example.json) の `event_url` を自分の公開 URL に置き換えて登録します。管理者が承認してインストールし、通知先チャンネルに Bot を追加してください。

「トークンを発行」で Access / Refresh Token、「接続情報」で Signing Secret を取得します。スコープは投稿・コマンド・本人向け UI・リンク展開・メンション用プロフィール読取だけです。通常の Zoo メッセージイベントは購読しません。

### 3. 通知先の許可を設定する

```sh
cd apps/github-bot
cp config.example.json config.json
cp .env.example .env
```

`config.json` に許可するリポジトリ・Installation ID・Zoo チャンネル ID を設定します。

```json
{
  "repositories": [
    {
      "repository": "your-org/your-repo",
      "installationId": 123456,
      "channels": ["zoo-channel-uuid"]
    }
  ],
  "users": { "github-login": "zoo-user-uuid" }
}
```

- 非公開リポジトリも、ここで指定したチャンネルの参加者へ通知・プレビューされます。管理者が共有先を確認して設定してください。コマンドからこの許可範囲を広げることはできません。設定変更は今後の配信に適用され、既存の投稿は Zoo の通常の履歴として残ります。
- `your-org/*` は、その Organization の許可済み Installation に属する現在・将来のリポジトリを対象にします。個別リポジトリに限定する場合はワイルドカードを使いません。
- `users` は任意です。GitHub の担当者・レビュー依頼・本文中のメンションを Zoo のユーザーへ対応付けます。現在の Zoo ユーザー名を取得して通知するため、名前変更後も同じユーザーを対象にします。
- 同じリポジトリに重なる規則は同じ Installation ID を使います。リポジトリ名・所有者を変更した場合は許可設定も更新します。

`.env` に Zoo の認証情報、GitHub App ID・鍵ファイル・Webhook Secret を設定します。`BOT_ENCRYPTION_KEY` は次のコマンドで生成した32バイトの鍵を設定します。これは SQLite 内の Zoo 資格情報を暗号化する鍵で、再起動後も保持してください。

```sh
openssl rand -base64 32
```

### 4. 起動する

```sh
npm ci --ignore-scripts
node --env-file=.env --experimental-strip-types src/main.ts
```

`POST /github/webhook` と `POST /zoo/events` を同じ Bot サーバーへ転送する HTTPS リバースプロキシを設定します。Zoo 本体の URL ではなく、Bot の公開 URL を GitHub App / Zoo App の両方に設定してください。Zoo の「配信先を検証」も利用できます。

初回は `ZOO_ACCESS_TOKEN` と `ZOO_REFRESH_TOKEN` が必要です。更新後の資格情報は暗号化して SQLite に保存します。再承認などで資格情報を発行し直した場合は `.env` の両方の値を変更して再起動すると置き換わります。別の Zoo App では別の DB を使ってください。

### Docker Compose

`.env` の `GITHUB_PRIVATE_KEY_PATH` にホスト側の鍵ファイルを指定し、UID 1000 のコンテナユーザーが読める権限にします。鍵は読み取り専用でマウントされます。

```sh
docker compose pull
docker compose up -d --no-build
```

公開イメージは `ghcr.io/zli-uoa/zoo2-github-bot` です。main の変更時に型チェック・テスト・コンテナ検証を通してから、コミット SHA タグと `latest` を発行します。GitHub Actions の `CI GitHub Bot` を main で手動実行して再発行することもできます。PR では公開せず、ビルドと検証だけを行います。初回の発行はこの PR のマージ後です。

バージョンを固定する場合は `.env` の `BOT_IMAGE` を `ghcr.io/zli-uoa/zoo2-github-bot:<main のコミット SHA>` に変更します。GHCR パッケージが非公開の場合は、`read:packages` 権限のある資格情報で `docker login ghcr.io` してから取得してください。

ホストの `127.0.0.1:3001` を HTTPS リバースプロキシに接続します。外部へ bind する必要がある場合は `BOT_BIND_ADDRESS` を設定します。SQLite は named volume に保存されます。DB・バックアップには非公開リポジトリの通知本文が含まれるため、同じ共有範囲で管理してください。

リポジトリルートから個別にビルドすることもできます。

```sh
docker build -f apps/github-bot/Dockerfile -t zoo-github-bot:local .
```

ローカルでビルドしたイメージを Compose で使う場合は、`apps/github-bot` で `BOT_IMAGE=zoo-github-bot:local docker compose up -d --build` を実行します。

## 購読する

通知は状態に応じた色のカードで表示します。PR のマージは紫、成功したデプロイは緑、失敗は赤で表示し、Status・Commit・Workflow などをリンク付きで配置します。コメントの編集や状態変更では、本文とカードを同じ投稿 ID で更新します。照合用のキーは小文字に統一しますが、投稿・カード・リンクには GitHub が返す組織名／ユーザー名・リポジトリ名の元の表記を使用します。

通知先チャンネルで実行します。購読状態・フィルター・スレッド設定は SQLite に保存され、再起動後も維持します。

```text
/github subscribe your-org/your-repo
/github subscribe your-org/your-repo reviews comments
/github subscribe your-org/your-repo commits:feature/*
/github subscribe your-org/your-repo +label:"priority: HIGH"
/github subscribe your-org/your-repo workflows:{name:"CI" event:"push","pull_request" branch:"main" actor:"alice"}
/github subscribe your-org/your-repo comments:"channel" reviews:"channel"
/github subscribe list features
/github unsubscribe your-org/your-repo comments reviews
/github unsubscribe your-org/your-repo
```

初回に通知種別を省略すると `issues pulls commits releases deployments` を有効にします。初回から通知種別を指定した場合は指定分だけを有効にし、既存の購読には追加します。ラベルだけを指定した初回購読は既定の通知種別にフィルターを適用します。

| 種別                | 通知と更新                                                                                                |
| ------------------- | --------------------------------------------------------------------------------------------------------- |
| `issues`            | 作成、クローズ／再オープン、編集、ラベル・担当・マイルストーン・ロックなどの変更                          |
| `pulls`             | 作成、draft / ready、レビュー依頼、タイトル等の編集、クローズ・マージ、キュー・自動マージ設定等の状態変更 |
| `comments`          | Issue / PR コメントの返信・編集・削除                                                                     |
| `reviews`           | レビュー結果・編集・取消、インラインコメントの返信・編集・削除、レビュー会話の解決／未解決                |
| `checks`            | Checks / commit status。PR を特定できる場合は PR の親にも反映                                             |
| `workflows`         | 実行開始・進行・完了。run attempt ごとの返信も保持                                                        |
| `deployments`       | 同じ deployment の準備・進行・成功・失敗等を追跡                                                          |
| `commits`           | 既定ブランチへの push。`commits:*` やブランチパターンで変更可能                                           |
| `branches` / `tags` | 作成・削除・再作成。親は現在の参照状態を反映                                                              |
| `releases`          | 公開、編集、リリース状態の変更、削除。draft は通知しない                                                  |
| `discussions`       | 作成・編集・回答済み状態、コメントの返信・編集・削除                                                      |

通常のコメント・レビューはスレッドだけに表示します。`comments:"channel"` / `reviews:"channel"` はチャンネルへの broadcast を有効にします。クローズ／再オープン・マージなどの状態変更もスレッドへの返信をチャンネルへ broadcast します。

`workflows` にフィルターを付けない場合は、既定ブランチを対象とする PR イベントを通知します。ワークフロー名・イベント・ブランチ・実行者は上の例で変更できます。ラベルフィルターは Issue / PR / コメント / レビューに適用します。既に追跡している親は、ラベルが外れた場合も最新の表示に更新します。

```text
/github subscribe your-org
/github settings threading off
/github settings threading on
/github reminders 09:00 Asia/Tokyo
/github reminders off
```

Organization 購読には管理者の `your-org/*` 許可が必要です。リマインダーは、そのチャンネルの購読先にある未対応のレビュー依頼を毎日指定時刻以降にまとめます。個人認証・個人 DM 配信は使いません。

## リンクのプレビュー

許可されたチャンネル内の `github.com` リンクを展開します。

- リポジトリ、Issue / PR、直接リンクした Issue コメント・インラインレビューコメント。
- commit、Actions run、release、Discussion。
- code blob の行範囲。ブランチ・タグ名に `/` があるリンクも扱います。
- 公開ユーザー・Organization プロフィール。

非公開リポジトリのリンクも通知と同じリポジトリ／チャンネル許可を確認します。Zoo 自身がリンクを取得するのではなく、Bot が GitHub API から取得して汎用カードを設定します。コードは UTF-8・200KB以下のファイルから最大20行、共有メッセージごとに最大3リンク、ref の解決は最大12候補です。

## 再送・順序・運用

- 両方の Webhook の署名を検証し、SQLite への記録が完了してから ACK します。GitHub は Delivery ID、Zoo は event ID で重複を排除します。
- 親・返信の Zoo 投稿 UUID は永続的な対象 ID から決定します。再起動や ACK 取りこぼし後も同じ投稿を再利用します。親の初回公開直後のクラッシュでも不要な「作成」返信を増やしません。
- 親の表示とコメント本文は GitHub API から現在値を読みます。古い Webhook が後から届いても、現在の PR 状態やコメント本文へ戻します。状態変更の返信は履歴として残し、到着順に並びます。
- GitHub のレート制限は `Retry-After` / rate reset に従います。取得の ETag と小さいレスポンスのメモリキャッシュも利用します。
- 受信済みジョブの通常の失敗は指数バックオフで最大12試行。失敗を使い切ったジョブは `dead` に残します。完了済みの受信履歴は30日保持します。
- ページ取得は100件×20ページが上限です。超過時は不完全な一覧を正しい結果として表示せず、失敗ジョブに残します。キューは未処理10,000件／256MiB、リクエストは25MiBまでで、満杯時は503を返します。
- 1 DB に1ワーカーです。DB の lease で同時起動を拒否し、異常停止後は30秒で復旧可能になります。複数環境・複数 Zoo App は DB を分けてください。
- `GET /health` は疎通確認用です。Zoo の認証が失効して処理停止した場合は503になります。再発行した資格情報を設定して再起動してください。

GitHub 側は失敗した Webhook を自動再送しません。サーバー停止や503で未受信だった配信は、復旧後に GitHub App の Recent Deliveries から redelivery してください。[公式の失敗配信の扱い](https://docs.github.com/en/webhooks/using-webhooks/handling-failed-webhook-deliveries)に従います。

失敗ジョブを確認する場合（本文や資格情報は出力しません）:

```sh
node --env-file=.env --experimental-strip-types src/admin.ts
```

原因を直した後、ワーカーを停止して再投入します。

```sh
node --env-file=.env --experimental-strip-types src/admin.ts retry github:DELIVERY_ID
```

## 仕様の照合

調査元は [GitHub + Slack の公式機能一覧](https://github.com/integrations/slack#features)、[通知のカスタマイズ](https://docs.github.com/en/integrations/how-tos/slack/customize-notifications)、[Webhook のイベント仕様](https://docs.github.com/en/webhooks/webhook-events-and-payloads)、[GitHub App のインストール認証](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation) です。コードは独自実装です。

| 公式連携の機能                                                      | この Bot                                                   |
| ------------------------------------------------------------------- | ---------------------------------------------------------- |
| リポジトリ／Organization 購読、通知の選別                           | 実装                                                       |
| Issue / PR の親カードを編集し、更新を返信に集約                     | 実装。Zoo の1段のスレッド内に GitHub のコメント URL を保持 |
| コメント・レビューの編集／削除、Checks の最新表示                   | 実装                                                       |
| Actions / デプロイの進行状況                                        | 実装                                                       |
| コード・Issue / PR 等のリンク展開                                   | 上記範囲で実装                                             |
| GitHub アカウントのメンション                                       | 管理者の GitHub login → Zoo user ID 対応で実装             |
| 定時レビュー依頼                                                    | チャンネル内の購読先に対して実装                           |
| Slack / Zoo から GitHub を更新                                      | 対象外。一方向通知の要件に従う                             |
| Slack 個人アプリ・Enterprise Grid・GitHub 側の個人／チーム用設定 UI | Zoo の外部サービスなので対象外                             |
| GHES / ghe.com / Copilot cloud agent                                | この PR の対象外。GitHub.com の通知を扱う                  |

実 GitHub App と本番 Zoo の秘密情報はテストに使用していません。テストは署名付きの実 HTTP リクエスト、SQLite の再起動、SDK と同じ投稿契約、GitHub API の制御可能な応答で行っています。

```sh
npm ci --ignore-scripts
npm run check
npm test
```
