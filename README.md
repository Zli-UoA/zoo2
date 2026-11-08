# Zoo2

**チームやコミュニティの会話を、自分たちのサーバーで。**

Zoo2 は、チャンネルとダイレクトメッセージを中心とした、セルフホスト可能なオープンソースのメッセージングツールです。スレッドでの議論、絵文字リアクション、ファイル共有、音声通話に対応し、Webhook や App・Bot で外部サービスとも連携できます。

[ローカルで試す](#ローカルで試す) · [ドキュメント](#ドキュメント) · [開発に参加する](#開発に参加する) · [Issues](https://github.com/Zli-UoA/zoo2/issues)

## 主な機能

- **チャンネルと DM** — 公開・非公開チャンネル、DM、スレッド返信、メンション、未読管理、入力中表示。
- **表現豊かなメッセージ** — 編集・削除、絵文字リアクション、添付ファイル、URL カード、MFM による装飾や数式表示。
- **リアルタイムの会話** — メッセージの同期と WebRTC による音声通話。
- **ブラウザーから利用** — ライト・ダークテーマ、多言語表示、PWA のインストール、Web Push。オフラインでは保存済みの会話を閲覧できます。
- **外部サービスとの連携** — Slack 互換 Webhook、App・Bot 向け API とイベント配信、GitHub 通知 Bot。
- **Home と ActivityPub** — タイムラインへの投稿とチャンネル連携。追加設定で外部サーバーとのフォローや投稿の送受信に対応します。対応範囲は [Home / ActivityPub](docs/activitypub.md) を参照してください。

## クライアント

通常の Web クライアントは React 版です。各クライアントの対応機能・設定方法は、それぞれのドキュメントを参照してください。

| クライアント | 案内 |
| --- | --- |
| Web / PWA | [Web クライアント](frontend/README.md)・[PWA とプッシュ通知](docs/pwa-and-push.md) |
| ターミナル | [TUI のインストールと使い方](tui/README.md) |
| デスクトップ（GPUI） | [ビルド・設定・操作方法](desktop/README.md) |
| Compose Web（実験版） | [切り替え方法と制限](docs/compose-multiplatform.md) |
| Android（Compose Multiplatform） | [ビルドと認証設定](docs/android.md) |

## ローカルで試す

開発用の認証で Web 版を起動する手順です。外部の認証サーバーは不要です。

### 必要なもの

- Git
- Flakes を有効にした Nix
- Docker Engine と Docker Compose v2（Docker デーモンを起動しておく）

Rust、Node.js、pnpm、Atlas などの開発ツールは、リポジトリの `nix develop` で用意します。

### 1. ソースと依存関係を準備する

```bash
git clone https://github.com/Zli-UoA/zoo2.git
cd zoo2
nix develop
cp .env.example .env
cp frontend/.env.example frontend/.env
make frontend-install
```

以降のコマンドはリポジトリのルートから実行します。別のターミナルでも、同じディレクトリで `nix develop` に入ってください。既存の環境を使う場合は、環境ファイルを上書きせず、設定を確認してください。

### 2. データベースと同期サービスを起動する

```bash
make db-up
make migrate
```

PostgreSQL、Electric、添付ファイル用の S3 互換ストレージ（RustFS）が起動し、データベースにマイグレーションを適用します。

### 3. バックエンドを起動する

```bash
make backend-dev
```

API は `http://localhost:8000` で起動します。このターミナルは起動したままにします。

### 4. 開発用ユーザーを作り、Web を起動する

別のターミナルで、初回のみ次を実行します。ユーザー ID は `frontend/.env.example` の `VITE_DEV_USER_ID` と同じ値です。

```bash
curl --fail-with-body http://localhost:8000/api/users \
  -H 'Content-Type: application/json' \
  -d '{"id":"00000000-0000-0000-0000-000000000001","user_name":"dev_user","display_name":"Dev User"}'
```

続けて Web の開発サーバーを起動します。

```bash
make frontend-dev
```

[http://localhost:5173](http://localhost:5173) を開き、チャンネルを作成してメッセージを送信できます。

終了するときは Web とバックエンドをそれぞれ `Ctrl+C` で停止し、`make db-down` でコンテナを停止します。データはボリュームに保持されます。

## セルフホストする

ローカル起動の構成は開発用です。公開環境では `ENVIRONMENT=production` を設定し、OIDC 認証、HTTPS、S3 互換ストレージ、Electric の認証・公開範囲を設定してください。本番モードでは開発用認証が無効になります。

- [インフラの構成と配備](infra/README.md)
- [Web の OIDC 認証設定](frontend/README.md#oidc-ログインauthz)
- [配布コンテナイメージ](docs/ghcr-images.md)

設定項目は [バックエンド・インフラ用の環境変数](.env.example) と [Web 用の環境変数](frontend/.env.example) にまとめています。

## ドキュメント

| 目的 | ドキュメント |
| --- | --- |
| メッセージの装飾・数式を使う | [MFM 対応表](docs/mfm.md) |
| 通知やオフライン閲覧を設定する | [PWA とプッシュ通知](docs/pwa-and-push.md)・[オフライン閲覧の範囲](docs/offline-windowed-messages.md) |
| Webhook で通知を送る | [Webhook 設定](docs/webhooks.md) |
| App・Bot を開発する | [App・Bot 基盤](docs/apps.md)・[GitHub 通知 Bot](apps/github-bot/README.md) |
| URL カードと画像プロキシを設定する | [URL カード](docs/link-previews.md) |
| Home や連合機能を使う | [Home / ActivityPub](docs/activitypub.md) |
| PR の検証環境を使う | [Preview Environment](docs/preview-environments.md) |
| 配備状況・履歴を確認する | [Flux の通知](docs/flux-status.md) |

## 開発に参加する

不具合報告や改善提案は [Issues](https://github.com/Zli-UoA/zoo2/issues)、修正は Pull Request で受け付けています。不具合の報告には、利用クライアント、再現手順、期待した動作、実際の動作を添えてください。

### リポジトリの構成

| ディレクトリ | 内容 |
| --- | --- |
| `backend/` | Rust / Axum による REST API、認証、Electric の Shape プロキシ |
| `frontend/` | React / TanStack による Web クライアント |
| `frontend-compose/` | Kotlin / Compose Multiplatform のクライアント |
| `client-core/` | Rust クライアントで共有する処理 |
| `tui/`・`desktop/` | ターミナル・デスクトップクライアント |
| `sdk/`・`apps/` | App 開発用 SDK と Bot |
| `infra/` | コンテナ、データベーススキーマ、マイグレーション、配備設定 |
| `docs/` | 機能別の仕様・設定・運用手順 |

Web の書き込みはバックエンドの REST API、リアルタイムの読み取りは認証・認可を行う Shape プロキシ経由の Electric を使います。データは PostgreSQL に保存します。

### 変更の確認

変更対象に合わせてチェックを実行してください。

```bash
make test-unit        # バックエンドのユニットテスト（DB 不要）
make frontend-check   # Web のフォーマット・lint・型チェック
make frontend-test    # Web のユニットテスト
make tui-check        # Rust 共通処理と TUI のチェック・テスト
```

バックエンドの E2E は `make test-e2e`（Docker が必要）、Web の E2E は起動済みの DB・Electric・バックエンドに対して `make frontend-e2e` で実行します。詳しい手順は [Web クライアントの README](frontend/README.md) と [開発ルール](AGENTS.md) を参照してください。

## ライセンス

Zoo2 は [GNU Affero General Public License v3.0（AGPL-3.0-only）](LICENSE) で公開されています。改変版をネットワーク経由で利用者に提供する場合は、その利用者が対応するソースコードを入手できるようにする必要があります。詳しい条件はライセンス本文を参照してください。

依存ライブラリ・フォント・同梱データなどの第三者の著作物には、それぞれのライセンスが適用されます。[Web のライセンス一覧](frontend/public/licenses/README.txt) など、各配布物の表記を参照してください。
