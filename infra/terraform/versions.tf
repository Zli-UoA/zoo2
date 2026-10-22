terraform {
  required_version = ">= 1.6.0"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }

  # NOTE: 本番運用時は S3 互換バックエンド (Cloudflare R2 等) または Terraform Cloud / S3 で state を管理します。
  # backend "s3" {}
}
