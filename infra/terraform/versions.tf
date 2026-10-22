terraform {
  required_version = ">= 1.6.0"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }

  # NOTE: 本番・ステージング運用時は R2 (S3 互換) または Terraform Cloud / S3 で remote backend を設定します。
  # state 用のバケット（例: zoo-terraform-state）はこの Terraform 管理外（手動または別系統）で作成してください。
  # backend "s3" {
  #   bucket                      = "zoo-terraform-state"
  #   key                         = "r2/terraform.tfstate"
  #   region                      = "auto"
  #   endpoint                    = "https://<ACCOUNT_ID>.r2.cloudflarestorage.com"
  #   skip_credentials_validation = true
  #   skip_metadata_api_check     = true
  #   skip_region_validation      = true
  #   skip_requesting_account_id  = true
  #   skip_s3_checksum            = true
  # }
}
