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

```bash
cd infra/terraform
cp terraform.tfvars.example terraform.tfvars
# terraform.tfvars に cloudflare_api_token と cloudflare_account_id を設定

terraform init
terraform plan
terraform apply
```

## GitHub Actions 自動化
`.github/workflows/terraform-r2.yaml` にて以下を実行します:
- PR 時: `terraform fmt -check`, `terraform validate`, `terraform plan`
- `main` ブランチへのプッシュ時（または手動実行 `workflow_dispatch`）: `terraform apply`
