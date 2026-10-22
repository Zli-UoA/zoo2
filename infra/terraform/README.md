# Cloudflare R2 Terraform 構成

Zoo アプリケーションのファイル・画像添付機能で使用する Cloudflare R2 バケットおよび関連ポリシー（CORS、ライフサイクル）をコード管理します。

## 管理対象リソース
- `zoo-attachments-prod` (本番環境用バケット, Location: `apac`)
- `zoo-attachments-stg` (ステージング / Preview環境用バケット, Location: `apac`)
- 各バケットの CORS ルール（ブラウザからの Direct PUT / GET 許可）
- 不完全マルチパートアップロードの自動破棄ルール（7日）

## 必要な権限
Cloudflare API Token に以下の権限を付与してください:
- `Account` > `R2 Storage` > `Edit`

## 手動実行手順

### 1. API トークンの設定 (環境変数推奨)
API トークンなどの秘密情報をファイルに保存せず誤コミットを防ぐため、CI と同様に環境変数で渡す運用を推奨します。

```bash
export TF_VAR_cloudflare_api_token="<YOUR_CLOUDFLARE_API_TOKEN>"
```

### 2. パラメータの設定 (`terraform.tfvars`)
アカウント ID やバケット名、CORS 許可オリジンなどの非秘密なパラメータは `terraform.tfvars` に記述します。
※ `terraform.tfvars` は `.gitignore` に登録されており、Git の追跡から除外されます。

```bash
cd infra/terraform
cp terraform.tfvars.example terraform.tfvars
# terraform.tfvars に cloudflare_account_id などを設定
```

### 3. 初期化と適用

```bash
terraform init
terraform plan
terraform apply
```

## Terraform State 管理方針 (重要)

- **Remote Backend**:
  本番・ステージング環境の自動 `apply` を CI で安全に行うためには、使い捨て runner で state が消失しないよう、State 管理専用の R2 バケット（例: `zoo-terraform-state`）を Terraform 管理外で事前に作成し、`versions.tf` の `backend "s3"` を有効化する必要があります。
- **CI 自動 apply の一時停止**:
  Remote backend が整うまでは、CI 上で apply すると tfstate が永続化されず次回以降にバケット重複エラーとなるため、GitHub Actions の `apply` ジョブは安全のため無効化（コメントアウト）しています。

## 既存バケットのインポート手順 (terraform import)

既に Cloudflare ダッシュボード等で R2 バケットを手動作成済みの場合は、以下のコマンドで Terraform 管理下にインポートできます。

```bash
cd infra/terraform

# 本番バケットのインポート (<account_id>/<bucket_name>)
terraform import cloudflare_r2_bucket.prod "<ACCOUNT_ID>/zoo-attachments-prod"

# ステージングバケットのインポート
terraform import cloudflare_r2_bucket.stg "<ACCOUNT_ID>/zoo-attachments-stg"

# 差分がないことを確認
terraform plan
```

## GitHub Actions 自動化
`.github/workflows/terraform-r2.yaml` にて以下を実行します:
- PR 時: `terraform fmt -check`, `terraform validate`, `terraform plan`（Secrets が注入されている場合のみ Plan を実行、fork PR では安全にスキップ）
- `main` マージ時: Remote backend 導入後に `apply` ジョブを有効化して自動適用予定

