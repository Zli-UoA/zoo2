# Infrastructure

フロント・バックエンド・認証用プロキシのコンテナと、PostgreSQL、ElectricSQL、Atlas migrationを管理します。

## 構成

```text
infra/
├── compose.stg.yaml         # ステージング構成
├── compose.stg.local.yaml   # 既存のローカル起動コマンド用 override
├── proxy/                   # Traefik のルーティング・forward-auth と検証
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

リポジトリルートで `--project-directory .` を指定してください。`POSTGRES_PASSWORD` と `BFF_VERIFY_SECRET` は必須です。

Rust の本番起動には `OIDC_ISSUER` と `OIDC_AUDIENCE`（未設定なら `OIDC_CLIENT_ID`）も必要です。既存の Kanidm クライアントに合わせて Coolify の環境変数に設定してください。`OIDC_JWKS_URL` は任意で、未設定なら Discovery を使います。OIDC 設定を省略すると Rust は起動を拒否します。フロント単体とモックを使ったプロキシテストには不要です。

```bash
POSTGRES_PASSWORD=staging-test-password BFF_VERIFY_SECRET=staging-test-verify-secret \
  docker compose --project-directory . -f infra/compose.stg.yaml \
  up -d --build --wait
```

公開入口は Traefik の `proxy` サービスで、コンテナ内の `8080` をホストの `127.0.0.1:18080` にマッピングします。ホスト側は `PROXY_PORT` で変更できます。Coolify 自身が使う `8080` とは分けています。フロント・Rust・DB・Electric のホストポートは公開しません。`PORT` は Rust のコンテナ内の待受ポートです（既定 `8000`）。既存の `compose.stg.local.yaml` を重ねた起動コマンドも、同じマッピングで引き続き利用できます。

フロントは TanStack Start を Nitro の Node.js サーバーとしてビルドし、非 root ユーザーで実行します。コンテナ内は `3000` で待ち受けます。フロントとプロキシのイメージ名は `FRONTEND_IMAGE`・`PROXY_IMAGE` で変更できます。フロントのヘルスチェックは `/` の HTTP 200、プロキシは内部専用の ping エンドポイントを確認します。

フロントの `HOST`・`PORT` と、Nitro が優先する `NITRO_HOST`・`NITRO_PORT` は `0.0.0.0:3000` に揃えています。Coolify で共有の `PORT` をバックエンド用に設定していても、フロントの待受ポートは変えません。Coolify 側でこのサービスの待受設定やヘルスチェックを上書きする場合も、接続先を `3000` に合わせてください。

### 認証とルーティング

[Issue #23 の決定](https://github.com/Zli-UoA/zoo2/issues/23#issuecomment-6220232883)に従い、ブラウザは同一オリジンの Cookie でアクセスします。Compose のフロントビルドでは `VITE_API_BASE_URL` を空文字に固定し、`/api/*`（Shape を含む）をプロキシから Rust へ直接転送します。それ以外の画面・ログイン・コールバックはフロントへ転送します。BFF は REST・Shape の本文を中継しません。

`/api/*` の転送前に BFF の verify を呼び、応答の `Authorization` を Rust へのリクエストに設定します。ブラウザの `Authorization`・`X-Dev-User` と認証判定を偽装できる forwarded ヘッダは除去します。API の応答は `Cache-Control: private, no-cache` とし、Authorization をブラウザに返しません。Shape の `electric-*`・`Vary` ヘッダは維持し、レスポンスを即時フラッシュします。プロキシと Rust 間の応答待ち時間（応答ヘッダが返るまで）は 120 秒です。Electric の long-poll 設定を延長する場合は、この設定と外側の Coolify プロキシも合わせて見直してください。応答を書き終えるまでの期限（entrypoint の `writeTimeout`）は設けていないので、SSE など長く続くストリームも切れません。

このインフラ構成の verify 接続先は `http://frontend:3000/auth/verify` です。パスは `BFF_VERIFY_PATH` で変更でき、公開入口からは同じパスとその配下へのアクセスを拒否します。verify のパス・ヘッダ名・応答形式の仕様は [Issue #23](https://github.com/Zli-UoA/zoo2/issues/23)（BFF 実装側）を基準とし、`infra/proxy/routes.yaml` はそれに合わせて追従します。verify を `/api/` 配下に置く場合も、この拒否ルールが優先されます。

`BFF_VERIFY_SECRET` はプロキシとフロント（BFF）で共有するランダムな値です。Coolify では両サービスに同じ値を設定してください。Traefik の設定テンプレートに埋め込むため、英数字のみ（例: `openssl rand -hex 32`）にしてください。

`BFF_VERIFY_PATH` は先頭を `/` にし、引用符を含めないでください。末尾の `/` はプロキシ内で除去し、認証先の URL と公開アクセスの拒否ルールに同じ正規化済みパスを使います。空文字や `/` だけの場合は `/auth/verify` にフォールバックします。

verify の実装には次の契約が必要です。

- Cookie からサーバ側セッションを解決し、必要な refresh をセッション単位で直列化した後、2xx と `Authorization: Bearer <access token>` を返す。本文やトークンをブラウザへ返さない。
- 未認証・セッション無効は 401、検証不能・ストア障害は非 2xx とする。プロキシは非 2xx や接続失敗を Rust に転送しない。
- verify は GET のサブリクエストとして呼ばれる。元のメソッド・URL は `X-Forwarded-Method`・`X-Forwarded-Uri`、Cookie・Origin・Sec-Fetch-Site・X-CSRF-Token はリクエストヘッダで渡される。元の本文は渡されない。
- `X-Verify-Secret` が `BFF_VERIFY_SECRET` と一致しないリクエストは拒否する（定数時間で比較する）。プロキシは verify へのサブリクエストにだけこのヘッダを付け、クライアントから来た同名ヘッダや Rust・フロントへのリクエストからは除去する。大文字小文字や `%2f` などのパス表現の差でプロキシのパス拒否ルールと解釈がずれた場合でも、ブラウザからの直接アクセスでトークンを返さないための多層防御です。BFF 実装時は `X-Verify-Secret` の未指定・不一致を拒否する単体テストも追加してください。
- 書き込みでは BFF が設定済みの公開オリジンと `Origin` を照合するか、CSRF トークンを検証する。Cookie の `SameSite=Lax` だけを認可根拠にしない。Cookie は `HttpOnly`・`Secure` とする。

Rust の JWT 検証・JIT は実装済みですが、BFF の verify・OIDC ログインは未実装です。ブラウザから API は利用可能になりません。今回のプロキシ検証ではモックを使い、実際の Kanidm ログイン・refresh・CSRF 実装と実 BFF コンテナを組み合わせた結合確認は BFF 実装後に行います。Rust は `ENVIRONMENT=production`・`DEV_AUTH_ENABLED=false` で起動します。

ステージングおよび初期構成では verify を SSR と同じ Nitro プロセスで動かします。verify は通常時に外部通信を行わずセッション参照だけで応答する実装とし、負荷が課題になった段階で別プロセスへの分離を検討します。また、Shape の初回同期では複数リクエストが重なるため、Traefik にパス一律の `rateLimit` は設けていません。レート制限は前段の Cloudflare と BFF の認証系エンドポイント単位で扱います。

BFF のセッションストアは未確定です。`frontend` は非 internal の `frontend` ネットワークにだけ参加しているので、internal な `backend` ネットワーク上の Postgres には到達できません。`frontend` ネットワークは BFF から Kanidm へ通信するため外向き通信を許可しています。将来 SSR で任意の外部 URL を取得する機能を追加する場合は、SSRF 対策やネットワーク分離を行ってください。セッションを Postgres に置く場合は、`frontend` を `backend` に入れないでください（Rust・Electric にも直接到達できてしまいます）。代わりに BFF 専用の internal ネットワークを作り、`frontend` と `postgres` だけを参加させてください。専用のストア（Redis など）を使う場合も、同じように BFF 専用のネットワークに置きます。

### Coolify

[Single Resource Tunnel の手順](https://coolify.io/docs/integrations/networking/cloudflare/tunnels/single-resource)を使います。Coolify の cloudflared テンプレートは `network_mode: host` なので、ホストの loopback ポートへ接続できます。

- Coolify のサービスの Domains は空にします。Tunnel が直接アプリのプロキシへ接続するため、Coolify 標準プロキシのドメインルーティングは使いません。
- Compose の `proxy` のポートマッピングは `127.0.0.1:18080:8080` です。Compose が定義しているため、Coolify 側で同じマッピングを重複追加する必要はありません。
- Cloudflare の公開ホスト名には利用するドメインを設定し、Path は空、Service は **HTTP / `127.0.0.1:18080`** にします。`PROXY_PORT` を変更した場合は接続先も合わせます。
- この接続先は HTTP なので Origin Server Name は不要です。ルーティングはパスで行い、HTTP Host Header の上書きも不要です。ブラウザからの公開 URL は HTTPS を使います。
- `frontend`・`backend` にドメインやホストポートを追加しないでください。Rust は内部ネットワークと Kanidm Discovery/JWKS 用の outbound ネットワークだけに参加します。

`PROXY_TRUSTED_IPS` には、アプリのプロキシから見える cloudflared の接続元 IP/CIDR を指定してください。ホストから Docker の公開ポートを経由するため、接続元が Docker ネットワークのゲートウェイ IP になる場合があります。既定の `127.0.0.1/32` でよいとは限りません。実際の接続元を確認して限定した値を設定し、`0.0.0.0/0` は指定しないでください。この範囲から来た forwarded ヘッダだけを信頼して、公開 HTTPS の scheme を BFF に渡します。

`X-Forwarded-Proto` を維持できるのは、信頼対象の cloudflared（Coolify 標準プロキシを経由させる構成ではそのプロキシ）が、ブラウザ由来の値を上書きして実際の公開 scheme を渡すことが前提です。ただし、Docker ブリッジ経由で接続元 IP が変わり `X-Forwarded-Proto` が `http` になった場合でも動作や安全性が崩れないよう、BFF の CSRF 判定・OIDC の `redirect_uri`・Cookie 設定では `X-Forwarded-Proto` に依存せず、BFF に設定する公開オリジンを基準にします。書き込み時の `Origin` ヘッダは `authRequestHeaders` でそのまま BFF に渡ります。

Tunnel からオリジンへの通信は HTTP です。認証実装時は BFF の Secure Cookie・公開 URL の扱いを確認し、オリジンまで TLS を使う場合は Coolify の [Full TLS 手順](https://coolify.io/docs/integrations/networking/cloudflare/tunnels/full-tls)に合わせて TLS の待受と証明書を別途設定します。

デプロイ後はホストで `curl -f http://127.0.0.1:18080/` を実行し、次に Tunnel の公開 URL から画面を確認します。Rust にプロキシ以外の公開経路がないこと、ブラウザから verify に直接アクセスできないことも確認してください。

フロントだけをビルド・起動確認する場合:

```bash
POSTGRES_PASSWORD=staging-test-password BFF_VERIFY_SECRET=staging-test-verify-secret \
  docker compose --project-directory . -f infra/compose.stg.yaml up -d --build --wait frontend
POSTGRES_PASSWORD=staging-test-password BFF_VERIFY_SECRET=staging-test-verify-secret \
  docker compose --project-directory . -f infra/compose.stg.yaml exec -T frontend \
  node -e "fetch('http://localhost:3000/').then(r => process.exit(r.status === 200 ? 0 : 1))"
```

### プロキシの自動検証

実際の Traefik とモックの BFF・Rust を使い、未認証・認証サービス障害・ヘッダ偽装・CSRF 判定用メタデータ・本文転送・Shape のストリーミングを確認します。テスト用の override は Coolify では使いません。

```bash
export POSTGRES_PASSWORD=proxy-test BFF_VERIFY_SECRET=proxy-test-verify-secret PROXY_PORT=18080
docker compose --project-directory . -f infra/compose.stg.yaml build proxy
docker compose --project-directory . -f infra/compose.stg.yaml -f infra/proxy/test/compose.yaml \
  up -d --no-build --no-deps --wait frontend backend proxy
nix develop --command node infra/proxy/test/check.mjs
docker compose --project-directory . -f infra/compose.stg.yaml -f infra/proxy/test/compose.yaml down
```
