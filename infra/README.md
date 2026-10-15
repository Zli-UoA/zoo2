# Infrastructure

PostgreSQL、ElectricSQL、Atlas migrationを管理します。

## 構成

```text
infra/
├── compose.yml              # PostgreSQLとElectricSQL
├── Makefile                 # DB操作とmigration操作
└── db/
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
docker compose --env-file .env -f infra/compose.yml ps
docker compose --env-file .env -f infra/compose.yml logs -f
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

PostgreSQL と ElectricSQL を起動し、migration を適用したうえで `cargo test` を実行します (Docker と atlas が必要)。

```bash
make test
```

CI (`.github/workflows/ci-backend.yaml`) の `test` ジョブでも同じ手順を実行します。
