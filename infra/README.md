# Infrastructure

PR ごとの k3s 検証環境は [Preview Environment の手順](../docs/preview-environments.md)を参照してください。旧ステージングの廃止は [#104](https://github.com/Zli-UoA/zoo2/issues/104) に分離し、以下の既存手順は維持します。

フロント・バックエンド・リバースプロキシのコンテナと、PostgreSQL、ElectricSQL、Atlas migrationを管理します。

## 構成

```text
infra/
├── compose.stg.yaml         # ステージング構成
├── compose.stg.local.yaml   # 既存のローカル起動コマンド用 override
├── Dockerfile-migrate-deploy # ステージング用 Atlas migration コンテナ
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

起動時は `postgres` が healthy になったあと、`migrate` コンテナ（`arigaio/atlas`、単一インスタンス前提）が `infra/db/migrations` を自動適用（`atlas migrate apply`）して終了し、その完了を待ってから `electric` と `backend` が起動します（このコンテナには Docker ソケットを持たせないため、`dev` DB を使う `migrate diff` / `validate` はローカルまたは CI で実行します）。

- **初期状態の DB**: `atlas_schema_revisions` が無い空の DB に対しては、初回の `migrate` 起動で全マイグレーションが順番に適用されます。
- **Atlas を介さず手動作成された既存 DB（baseline）**: もし `atlas_schema_revisions` が無いまま手動でテーブルだけ作成されている場合は、そのままでは初回マイグレーションが既存テーブルと衝突して停止します。一度だけ `--baseline <適用済みバージョン>`（例: `20261015110156`）を付けて `atlas migrate apply` を実行するか、ステージングで不要なデータであればボリュームを初期化してから再起動してください。
- **失敗時の確認・復旧**: マイグレーションが途中で失敗すると、後続の `electric` / `backend` は起動せず（fail-closed）、該当バージョンが dirty 状態になることがあります。その場合は `docker compose --project-directory . -f infra/compose.stg.yaml run --rm migrate migrate status --config file:///db/atlas.hcl --env local` で状態を確認し、原因解消後に再実行してください。

フロントは TanStack Start の SPA モードでビルドした静的ファイル（`dist/client`）を `nginxinc/nginx-unprivileged:1.30.6-alpine` で配信します。コンテナ内は非 root ユーザーで `3000` 番ポートを待ち受けます。設定は `infra/frontend-nginx.conf` です。未知のパスは `/index.html` にフォールバックしますが、`/assets/` 配下の存在しないファイルは 404 を返します（デプロイ後に古いタブが削除済みの chunk を読み込んだとき、HTML を返さないため）。`/assets/` のハッシュ付きファイルは `immutable` で長期キャッシュし、`index.html` は `no-cache` で毎回再検証させます。あわせて `X-Content-Type-Options: nosniff`・`Referrer-Policy: strict-origin-when-cross-origin`・`X-Frame-Options: DENY` と `Content-Security-Policy` を付与します。CSP は `infra/frontend-nginx.conf` をテンプレートとして起動時に envsubst し、ビルド成果物の `index.html` に含まれるインライン script の SHA-256（`infra/csp-hashes.mjs` がビルド時に生成）を `script-src` に、`connect-src` には同一オリジンと `CSP_CONNECT_SRC` を設定します（詳細は下記「CSP」）。フロント・プロキシ・マイグレーションのイメージ名は `FRONTEND_IMAGE`・`PROXY_IMAGE`・`MIGRATE_IMAGE` で変更できます。フロントのヘルスチェックは `/` の HTTP 200、プロキシは内部専用の ping エンドポイントを確認します。

### 認証とルーティング

ブラウザ（SPA）は Kanidm から取得した JWT を `Authorization: Bearer <token>` ヘッダに付与して API を呼び出します。Compose のフロントビルドでは `VITE_API_BASE_URL` を空文字に固定し、`/api/*`（Shape を含む）をプロキシから Rust へ直接転送します。それ以外のパスはフロントの静的配信コンテナへ転送します。

`/api/*` の転送ではクライアントの `Authorization` ヘッダをそのまま Rust へ通し、開発用の `X-Dev-User` ヘッダはプロキシで除去します。API の応答には `Cache-Control: private, no-cache` を付与します。Shape の `electric-*`・`Vary` ヘッダは維持し、レスポンスを即時フラッシュします。プロキシと Rust 間の応答待ち時間（応答ヘッダが返るまで）は 120 秒です。Electric の long-poll 設定を延長する場合は、この設定と外側の Coolify プロキシも合わせて見直してください。応答を書き終えるまでの期限（entrypoint の `writeTimeout`）は設けていないので、SSE など長く続くストリームも切れません。

Rust は `ENVIRONMENT=production`・`DEV_AUTH_ENABLED=false` で起動し、受け取った Bearer トークンを JWKS で検証します。また、Shape の初回同期では複数リクエストが重なるため、Traefik にパス一律の `rateLimit` は設けていません。レート制限は前段の Cloudflare 等で扱います。

#### フロントの OIDC PKCE ログイン

SPA は `VITE_OIDC_ISSUER`・`VITE_OIDC_CLIENT_ID`・`VITE_OIDC_REDIRECT_URI`（既定 `${origin}/auth/callback`）・`VITE_OIDC_SCOPE`（既定 `openid profile email`）・`VITE_OIDC_POST_LOGOUT_REDIRECT_URI` をビルド時に埋め込み、Authorization Code + PKCE (S256) でログインします。未設定（空）なら従来どおり dev の `X-Dev-User` で動きます。値は `infra/compose.stg.yaml` の frontend の build args に渡します。`state` / `nonce` / `code_verifier` は `sessionStorage` に一時保存し、コールバックで使い切りにします。トークン本体はメモリのみに置きます。

`VITE_OIDC_*` は **ビルド時に静的ファイルへ埋め込まれる**ため、変更したら `frontend` イメージを再ビルドしてください（コンテナの再起動だけでは反映されません）。

| Variable | 設定先 | 必須 | 説明 |
| --- | --- | --- | --- |
| `OIDC_ISSUER` | backend | 必須 | Kanidm の issuer。トークンの `iss` と完全一致させる |
| `OIDC_AUDIENCE` | backend | 必須 | 受け付ける `aud`。未設定時は `OIDC_CLIENT_ID` にフォールバック |
| `OIDC_JWKS_URL` | backend | 任意 | 未設定なら Discovery から解決 |
| `VITE_OIDC_ISSUER` | frontend build | 必須 | `OIDC_ISSUER` と同じ値 |
| `VITE_OIDC_CLIENT_ID` | frontend build | 必須 | Kanidm の Public client ID |
| `VITE_OIDC_REDIRECT_URI` | frontend build | 任意 | 既定 `${origin}/auth/callback`。ステージングでは公開 URL を明示 |
| `VITE_OIDC_SCOPE` | frontend build | 任意 | 既定 `openid profile email` |
| `VITE_OIDC_POST_LOGOUT_REDIRECT_URI` | frontend build | 任意 | 既定 `${origin}/` |
| `CSP_CONNECT_SRC` | frontend runtime | OIDC 利用時は必須 | Kanidm のオリジン（`connect-src` に追加） |

バックエンド側の `OIDC_ISSUER` / `OIDC_AUDIENCE` と、トークンの `iss` / `aud` / `typ` を一致させてください。

リロード後は IdP セッションを使って authorize へ再リダイレクトし（サイレント再ログイン）、期限前には refresh_token で更新します。Kanidm は refresh_token の再利用でセッションを失効させるため、同時更新は 1 リクエストに集約し、更新後の refresh_token に置き換えます。401 と refresh 失敗は再ログインに倒します。ログアウトはメモリのトークン破棄までで、Kanidm は RP-initiated logout（`end_session_endpoint`）を公開していないため、Kanidm セッション自体を終えるには Kanidm 側でログアウトしてください。

SPA は IdP の Discovery / token エンドポイントをブラウザから直接叩くため、IdP 側で SPA のオリジンに CORS を許可する必要があります（Kanidm は OAuth2 クライアントに設定した redirect origin に応じて CORS を返します）。CSP の `connect-src` には `CSP_CONNECT_SRC` で Kanidm のオリジンを追加します。

#### CSP

`infra/frontend-nginx.conf` は nginx 公式イメージの envsubst 用テンプレート（`/etc/nginx/templates/default.conf.template`）として配置し、コンテナ起動時に次の値を埋め込みます。

| Variable | 設定元 | 用途 |
| --- | --- | --- |
| `CSP_SCRIPT_SRC` | ビルド時に自動生成 | `index.html` のインライン `<script>` の SHA-256。手動設定しない |
| `CSP_CONNECT_SRC` | frontend の実行時環境変数 | `connect-src` に追加するオリジン（空白区切り）。既定は空で `'self'` のみ |

`CSP_CONNECT_SRC` には Kanidm の **issuer と token エンドポイントのオリジン**を指定します（例: `https://idm.example.com`）。Kanidm では通常どちらも同じオリジンです。JWKS は Rust が取得するためブラウザの CSP には不要ですが、リバースプロキシ等で別オリジンに置いている場合でも Rust 側の到達性だけを確認してください。`VITE_API_BASE_URL` を空以外にして API を別オリジンへ向ける構成にした場合は、そのオリジンも `CSP_CONNECT_SRC` に追加してください（Compose のステージング構成では `/api/*` を同一オリジンで Traefik から Rust へ転送するため不要です）。

`img-src` は `'self' data: https://www.gstatic.com` に固定しています。`https://www.gstatic.com` は Emoji Kitchen の合成画像（#82）をブラウザから直接取得するためです。新しい外部画像オリジンを追加する場合は、`/` と `/assets/` の両方の `img-src` と `container` ジョブの検査を更新してください。

`script-src` は `'unsafe-inline'` を使わず、ビルド時のハッシュで許可します。ハッシュは `infra/csp-hashes.mjs` が `pnpm build` 後の `index.html` から生成し、`infra/frontend-entrypoint.sh` が起動時に `CSP_SCRIPT_SRC` として渡します。`index.html` やテーマ初期化 script を変更した場合は、必ずイメージを再ビルドしてください。`style-src` の `'unsafe-inline'` は React の style 属性に必要なため残しています。

CSP の確認例（ステージング）:

```bash
curl -fsSI https://<公開ホスト>/ | grep -i content-security-policy
# connect-src に Kanidm のオリジンが入っていること
```

さらにブラウザの DevTools Console に CSP 違反（`Refused to execute inline script` 等）が出ないこと、ログイン → チャンネル作成 → メッセージ送信 → 別タブ/再読み込みで反映、を確認します。CI の `container` ジョブは既定の CSP と `CSP_CONNECT_SRC` の展開に加え、ビルド済み nginx に対して Playwright（`pnpm test:e2e:csp`）を実行し、CSP 違反が 0 件であることとテーマ初期化 script の実行を検証します。

#### 実 Kanidm 接続時の確認項目

モック IdP での自動テストは `pnpm test:e2e:oidc`（CI: `.github/workflows/ci-frontend-auth-e2e.yaml`）で通りますが、実 Kanidm では次を確認してください。

- Kanidm の OAuth2 クライアントを **Public client + PKCE (S256)** にし、redirect URI を公開 URL の `${origin}/auth/callback` に一致させる（`VITE_OIDC_REDIRECT_URI` を明示する場合はその値）。
- SPA のオリジンからの Discovery / token への CORS を許可する。
- `VITE_OIDC_ISSUER` を token の `iss` と完全一致させる（末尾スラッシュの有無を含む）。`VITE_OIDC_CLIENT_ID` と Rust の `OIDC_AUDIENCE` を一致させる。
- `VITE_OIDC_SCOPE` は `openid profile email` を基本とする。Kanidm 側で scope を絞る場合は `preferred_username` / `name` / `email` がトークンに含まれることを確認する。
- `CSP_CONNECT_SRC` に Kanidm のオリジンを設定する。
- Rust を `ENVIRONMENT=production`・`DEV_AUTH_ENABLED=false` で起動し、`/api/users/me`・Shape・REST が Bearer で通ることを確認する。
- リロード後のサイレント再ログイン、refresh_token のローテーション、401 時の再ログインがループしないことを確認する。

> **メモ:** 現在ユーザー（内部 `users.id`）は SPA が起動時に `GET /api/users/me`（Bearer）で解決し、自分の投稿判定・参加状態・メンバー表示・退出の本人確認に使います（#49）。解決できるまでは参加状態や作成・退出の操作を出さず、解決に失敗した場合は再読み込みまたは画面遷移で再試行します。`VITE_OIDC_*` を有効にする際は、この解決と Shape / REST の Bearer がバックエンドで通ることをステージングで確認してください（#50）。

### Coolify

[Single Resource Tunnel の手順](https://coolify.io/docs/integrations/networking/cloudflare/tunnels/single-resource)を使います。Coolify の cloudflared テンプレートは `network_mode: host` なので、ホストの loopback ポートへ接続できます。

- Coolify のサービスの Domains は空にします。Tunnel が直接アプリのプロキシへ接続するため、Coolify 標準プロキシのドメインルーティングは使いません。
- Compose の `proxy` のポートマッピングは `127.0.0.1:18080:8080` です。Compose が定義しているため、Coolify 側で同じマッピングを重複追加する必要はありません。
- Cloudflare の公開ホスト名には利用するドメインを設定し、Path は空、Service は **HTTP / `127.0.0.1:18080`** にします。`PROXY_PORT` を変更した場合は接続先も合わせます。
- リクエストは Tunnel → `127.0.0.1:18080` → Traefik の順で入ります。`/api` と `/api/*` は priority 100 の `api` ルーターで Rust へ、それ以外はフロントへ流れます。Cloudflare のレート制限は `/api/*` を含む公開ホスト全体に適用し、Tunnel 以外の公開経路を作らないでください。
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
