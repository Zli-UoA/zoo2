# Zoo2

Zoo2 は、セルフホストできるオープンソースのメッセージングツールです。チャンネルや DM での会話に加え、Webhook や App・Bot で外部サービスと連携できます。

## 主な機能

- 公開・非公開チャンネル、DM、スレッド、メンション、未読管理
- 絵文字リアクション、ファイル共有、URL カード、MFM・数式表示
- リアルタイム同期、入力中表示、音声通話
- 多言語表示、ライト・ダークテーマ、PWA、プッシュ通知、保存済みの会話のオフライン閲覧
- Home タイムラインとチャンネル連携、ActivityPub による外部サーバーとの送受信（追加設定が必要）

## ローカルで試す

Git、Flakes を有効にした Nix、起動済みの Docker Engine と Docker Compose v2 が必要です。Rust・Node.js・pnpm・Atlas は `nix develop` で用意します。外部の認証サーバーは不要です。

### セットアップ

```bash
git clone https://github.com/Zli-UoA/zoo2.git
cd zoo2
nix develop
cp .env.example .env
cp frontend/.env.example frontend/.env
make frontend-install
make db-up
make migrate
```

既存の `.env` がある場合は上書きせず、設定を確認してください。

### バックエンドの起動

```bash
make backend-dev
```

API は `http://localhost:8000` で起動します。

### Web の起動

別のターミナルでリポジトリのルートに移動し、`nix develop` に入ってください。初回のみ開発用ユーザーを作成してください。ID は `frontend/.env` の `VITE_DEV_USER_ID` に合わせます。

```bash
curl --fail-with-body http://localhost:8000/api/users \
  -H 'Content-Type: application/json' \
  -d '{"id":"00000000-0000-0000-0000-000000000001","user_name":"dev_user","display_name":"Dev User"}'

make frontend-dev
```

[http://localhost:5173](http://localhost:5173) を開いてチャンネルを作成できます。

終了時は Web とバックエンドをそれぞれ `Ctrl+C` で止め、`make db-down` を実行します。データは保持されます。

## セルフホスト

公開環境では `ENVIRONMENT=production` を設定し、OIDC 認証、HTTPS、S3 互換ストレージ、Electric の認証・公開範囲を設定してください。開発用認証は本番モードで無効になります。

手順は [インフラ](infra/README.md)、[OIDC 設定](frontend/README.md#oidc-ログインauthz)、[配布イメージ](docs/ghcr-images.md) を参照してください。環境変数は [.env.example](.env.example) と [frontend/.env.example](frontend/.env.example) に記載しています。

## ドキュメント

Web は React で開発しています。[Compose Web の実験は終了](docs/compose-multiplatform.md)し、Android の開発は継続しています。

| 内容 | リンク |
| --- | --- |
| クライアント | [Web / React](frontend/README.md)・[TUI](tui/README.md)・[デスクトップ](desktop/README.md)・[Android](docs/android.md) |
| メッセージ表示 | [MFM](docs/mfm.md)・[URL カード](docs/link-previews.md) |
| 通知・オフライン | [PWA とプッシュ通知](docs/pwa-and-push.md)・[オフライン閲覧の範囲](docs/offline-windowed-messages.md) |
| 外部連携 | [Slack 互換 Webhook](docs/webhooks.md)・[App・Bot](docs/apps.md)・[GitHub Bot](apps/github-bot/README.md) |
| タイムライン・連合 | [Home / ActivityPub の設定と対応範囲](docs/activitypub.md) |
| 開発・運用 | [開発ルール](AGENTS.md)・[Preview 環境](docs/preview-environments.md)・[配備状況](docs/flux-status.md) |

## 開発への参加

不具合報告・改善提案は [Issues](https://github.com/Zli-UoA/zoo2/issues)、修正は Pull Request で受け付けています。不具合には利用クライアントと再現手順、期待した動作と実際の動作を添えてください。

変更対象に合わせてチェックを実行してください。

```bash
make test-unit        # バックエンド（DB 不要）
make frontend-check   # Web のフォーマット・lint・型チェック
make frontend-test    # Web のユニットテスト
make tui-check        # Rust 共通処理と TUI
```

E2E の実行条件は [Makefile](Makefile) と [Web の README](frontend/README.md) を参照してください。

## ライセンス

Zoo2 は [AGPL-3.0-only](LICENSE) で公開しています。改変版をネットワーク経由で提供する場合は、利用者が対応するソースコードを入手できるようにする必要があります。

第三者のライブラリ・フォント・データには、それぞれのライセンスが適用されます。[Web のライセンス一覧](frontend/public/licenses/README.txt) など、各配布物の表記を参照してください。
