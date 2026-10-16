# Infrastructure

フロント・バックエンド・リバースプロキシのコンテナと、PostgreSQL、ElectricSQL、Atlas migrationを管理します。

## 構成

```text
infra/
├── compose.stg.yaml         # ステージング構成
├── compose.stg.local.yaml   # 既存のローカル起動コマンド用 override
├── frontend-nginx.conf      # フロントの静的配信 (nginx) の設定
├── proxy/                   # Traefik のルーティングと検証
├── Makefile                 # DB操作とmigration操作
└── db/
    ├── compose.yml          # PostgreSQLとElectricSQL
    ├── atlas.hcl            # Atlas設定
    ├── schema.sql           # 期待するデータベーススキーマ
    └── migrations/          # バージョン管理されたmigration
```

| Service | URL / Port | 用途 |
| --- | --- | --- |
| PostgreSQL 16 | `localhost:5432` | アプリケーションDB |
| ElectricSQL 1.8.1 | `http://localhost:3000` | PostgreSQLの変更をShape APIで配信 |

どちらのポートもホストのloopback interfaceだけに公開されます。ElectricSQLはローカル開発用のinsecureモードで動作するため、この構成をそのまま本番環境では使用しないでください。

## 必要なもの

- Docker Engine
- Docker Compose v2
- Nix

リポジトリのルートで開発環境に入ります。

```bash
nix develop
```

## 環境変数

デフォルト値のままなら`.env`なしでも起動できます。設定を変更する場合は、リポジトリのルートに`.env`を作成します。

```bash
cp .env.example .env
```

主な設定値は次のとおりです。

| Variable | Default | Description |
| --- | --- | --- |
| `POSTGRES_USER` | `root` | PostgreSQLユーザー |
| `POSTGRES_PASSWORD` | `password` | PostgreSQLパスワード |
| `POSTGRES_DB` | `zoo` | データベース名 |
| `POSTGRES_PORT` | `5432` | ホスト側のPostgreSQLポート |
| `ELECTRIC_VERSION` | `1.8.1` | ElectricSQL Dockerイメージのバージョン |
| `ELECTRIC_PORT` | `3000` | ホスト側のElectricSQLポート |
| `DATABASE_URL` | PostgreSQLのローカルURL | アプリケーションとAtlasの接続先 |
| `ELECTRIC_URL` | `http://localhost:3000` | ElectricSQLのHTTP API |

## 起動

リポジトリのルートから実行します。

```bash
make db-up
```

PostgreSQLがhealthyになった後でElectricSQLが起動します。状態とログはDocker Composeで確認できます。Docker Composeを直接実行する場合は、先に`.env`を作成してください。

```bash
docker compose --env-file .env -f infra/db/compose.yml ps
docker compose --env-file .env -f infra/db/compose.yml logs -f
```

ElectricSQLのhealth endpointは次のコマンドで確認できます。

```bash
curl http://localhost:3000/v1/health
```

正常時のレスポンス：

```json
{"status":"active"}
```

## Migration

初回起動後、Atlasでmigrationを適用します。

```bash
make migrate
```

適用状況を確認するには次を実行します。

```bash
make migrate-status
```

migrationファイルの整合性を検証するには次を実行します。

```bash
make migrate-validate
```

スキーマを変更するときは`infra/db/schema.sql`を編集し、名前を指定してmigrationを生成します。

```bash
make migrate-diff name=add_example
```

生成されたSQLを確認してから適用してください。

```bash
make migrate
```

## 停止と初期化

コンテナを停止・削除します。PostgreSQLのデータボリュームは保持されます。

```bash
make db-down
```

サービスを再起動します。

```bash
make db-restart
```

データベースを完全に初期化する場合は、コンテナとPostgreSQLボリュームを削除します。

```bash
make db-reset
```

`db-reset`はローカルの全データを削除し、復元できません。再作成後は`make db-up`と`make migrate`を実行してください。

## テスト (E2E)

E2E (`backend/tests/`) は PostgreSQL と ElectricSQL を起動し、migration を適用したうえで実行します (Docker と atlas が必要)。

```bash
make test-unit  # ユニットテスト (DB 不要)
make test-e2e   # E2E
make test       # 両方
```

CI (`.github/workflows/ci-backend.yaml`) では `unit-test` ジョブ (DB 不要) と `e2e` ジョブ (上記と同じ手順) に分けて実行します。

## ステージング

DBサービスは Coolify が `extends` や `!reset` を扱えないため、`compose.stg.yaml` に展開して定義しています（ホスト側ポート公開なし）。`db/compose.yml` を変更したら、こちらも合わせて更新してください。

リポジトリルートで `--project-directory .` を指定してください。`POSTGRES_PASSWORD` は必須です。

Rust の本番起動には `OIDC_ISSUER` と `OIDC_AUDIENCE`（未設定なら `OIDC_CLIENT_ID`）も必要です。既存の Kanidm クライアントに合わせて Coolify の環境変数に設定してください。`OIDC_JWKS_URL` は任意で、未設定なら Discovery を使います。OIDC 設定を省略すると Rust は起動を拒否します。フロント単体とモックを使ったプロキシテストには不要です。

```bash
POSTGRES_PASSWORD=staging-test-password \
  docker compose --project-directory . -f infra/compose.stg.yaml \
  up -d --build --wait
```

公開入口は Traefik の `proxy` サービスで、コンテナ内の `8080` をホストの `127.0.0.1:18080` にマッピングします。ホスト側は `PROXY_PORT` で変更できます。Coolify 自身が使う `8080` とは分けています。フロント・Rust・DB・Electric のホストポートは公開しません。`PORT` は Rust のコンテナ内の待受ポートです（既定 `8000`）。既存の `compose.stg.local.yaml` を重ねた起動コマンドも、同じマッピングで引き続き利用できます。

フロントは TanStack Start の SPA モードでビルドした静的ファイル（`dist/client`）を `nginxinc/nginx-unprivileged:1.30.6-alpine` で配信します。コンテナ内は非 root ユーザーで `3000` 番ポートを待ち受けます。設定は `infra/frontend-nginx.conf` です。未知のパスは `/index.html` にフォールバックしますが、`/assets/` 配下の存在しないファイルは 404 を返します（デプロイ後に古いタブが削除済みの chunk を読み込んだとき、HTML を返さないため）。`/assets/` のハッシュ付きファイルは `immutable` で長期キャッシュし、`index.html` は `no-cache` で毎回再検証させます。あわせて `X-Content-Type-Options: nosniff`・`Referrer-Policy: strict-origin-when-cross-origin`・`X-Frame-Options: DENY` を付与します（`Content-Security-Policy` は OIDC PKCE 実装時に Kanidm への `connect-src` 等が確定した段階で設定します）。フロントとプロキシのイメージ名は `FRONTEND_IMAGE`・`PROXY_IMAGE` で変更できます。フロントのヘルスチェックは `/` の HTTP 200、プロキシは内部専用の ping エンドポイントを確認します。

### 認証とルーティング

ブラウザ（SPA）は Kanidm から取得した JWT を `Authorization: Bearer <token>` ヘッダに付与して API を呼び出します。Compose のフロントビルドでは `VITE_API_BASE_URL` を空文字に固定し、`/api/*`（Shape を含む）をプロキシから Rust へ直接転送します。それ以外のパスはフロントの静的配信コンテナへ転送します。

`/api/*` の転送ではクライアントの `Authorization` ヘッダをそのまま Rust へ通し、開発用の `X-Dev-User` ヘッダはプロキシで除去します。API の応答には `Cache-Control: private, no-cache` を付与します。Shape の `electric-*`・`Vary` ヘッダは維持し、レスポンスを即時フラッシュします。プロキシと Rust 間の応答待ち時間（応答ヘッダが返るまで）は 120 秒です。Electric の long-poll 設定を延長する場合は、この設定と外側の Coolify プロキシも合わせて見直してください。応答を書き終えるまでの期限（entrypoint の `writeTimeout`）は設けていないので、SSE など長く続くストリームも切れません。

Rust は `ENVIRONMENT=production`・`DEV_AUTH_ENABLED=false` で起動し、受け取った Bearer トークンを JWKS で検証します。また、Shape の初回同期では複数リクエストが重なるため、Traefik にパス一律の `rateLimit` は設けていません。レート制限は前段の Cloudflare 等で扱います。

#### フロントの OIDC PKCE ログイン

SPA は `VITE_OIDC_ISSUER`・`VITE_OIDC_CLIENT_ID`・`VITE_OIDC_REDIRECT_URI`（既定 `${origin}/auth/callback`）・`VITE_OIDC_SCOPE`（既定 `openid profile email`）・`VITE_OIDC_POST_LOGOUT_REDIRECT_URI` をビルド時に埋め込み、Authorization Code + PKCE (S256) でログインします。未設定（空）なら従来どおり dev の `X-Dev-User` で動きます。値は `infra/compose.stg.yaml` の frontend の build args に渡します。`state` / `nonce` / `code_verifier` は `sessionStorage` に一時保存し、コールバックで使い切りにします。トークン本体はメモリのみに置きます。

リロード後は IdP セッションを使って authorize へ再リダイレクトし（サイレント再ログイン）、期限前には refresh_token で更新します。Kanidm は refresh_token の再利用でセッションを失効させるため、同時更新は 1 リクエストに集約し、更新後の refresh_token に置き換えます。401 と refresh 失敗は再ログインに倒します。ログアウトはメモリのトークン破棄までで、Kanidm は RP-initiated logout（`end_session_endpoint`）を公開していないため、Kanidm セッション自体を終えるには Kanidm 側でログアウトしてください。

SPA は IdP の Discovery / token エンドポイントをブラウザから直接叩くため、IdP 側で SPA のオリジンに CORS を許可する必要があります（Kanidm は OAuth2 クライアントに設定した redirect origin に応じて CORS を返します）。`Content-Security-Policy` の `connect-src` は Kanidm の確定後に #50 で設定します。

### Coolify

[Single Resource Tunnel の手順](https://coolify.io/docs/integrations/networking/cloudflare/tunnels/single-resource)を使います。Coolify の cloudflared テンプレートは `network_mode: host` なので、ホストの loopback ポートへ接続できます。

- Coolify のサービスの Domains は空にします。Tunnel が直接アプリのプロキシへ接続するため、Coolify 標準プロキシのドメインルーティングは使いません。
- Compose の `proxy` のポートマッピングは `127.0.0.1:18080:8080` です。Compose が定義しているため、Coolify 側で同じマッピングを重複追加する必要はありません。
- Cloudflare の公開ホスト名には利用するドメインを設定し、Path は空、Service は **HTTP / `127.0.0.1:18080`** にします。`PROXY_PORT` を変更した場合は接続先も合わせます。
- この接続先は HTTP なので Origin Server Name は不要です。ルーティングはパスで行い、HTTP Host Header の上書きも不要です。ブラウザからの公開 URL は HTTPS を使います。
- `frontend`・`backend` にドメインやホストポートを追加しないでください。`frontend` は `frontend` ネットワークにだけ参加します（proxy のホストポート公開に使うため、このネットワークは `internal` にしていません）。Rust は内部ネットワークと Kanidm Discovery/JWKS 用の outbound ネットワークだけに参加します。
- 現在は Traefik・Rust ともに `X-Forwarded-*` ヘッダを認証やルーティングに使わないため、`forwardedHeaders.trustedIPs` は設定していません。将来、監査ログや IP 単位のレート制限などでクライアント IP を参照する必要が出た場合は、`proxy` の command に `--entrypoints.web.forwardedheaders.trustedips` を追加し、`proxy` から見える cloudflared の接続元 IP/CIDR（Docker ブリッジのゲートウェイ IP 等、`0.0.0.0/0` は不可）に限定して設定してください。

デプロイ後はホストで `curl -f http://127.0.0.1:18080/` を実行し、次に Tunnel の公開 URL から画面を確認します。Rust にプロキシ以外の公開経路がないことも確認してください。

フロントだけをビルド・起動確認する場合:

```bash
POSTGRES_PASSWORD=staging-test-password \
  docker compose --project-directory . -f infra/compose.stg.yaml up -d --build --wait frontend
POSTGRES_PASSWORD=staging-test-password \
  docker compose --project-directory . -f infra/compose.stg.yaml exec -T frontend \
  wget -q -O /dev/null http://127.0.0.1:3000/channels/test
```

### プロキシの自動検証

実際の Traefik とモックのフロント・Rust を使い、`Authorization` ヘッダの透過転送、`X-Dev-User` の除去、本文転送、Shape のストリーミングを確認します。テスト用の override は Coolify では使いません。

```bash
export POSTGRES_PASSWORD=proxy-test PROXY_PORT=18080
docker compose --project-directory . -f infra/compose.stg.yaml build proxy
docker compose --project-directory . -f infra/compose.stg.yaml -f infra/proxy/test/compose.yaml \
  up -d --no-build --no-deps --wait frontend backend proxy
nix develop --command node infra/proxy/test/check.mjs
docker compose --project-directory . -f infra/compose.stg.yaml -f infra/proxy/test/compose.yaml down
```
