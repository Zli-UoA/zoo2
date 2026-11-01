terraform {
  required_version = ">= 1.10.0"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }

  # state は Zli アカウントの共有バケット zoo-terraform-state に置く（zoo2-infra・zeee と key を分けて共用）。
  # 認証は AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY（R2 の S3 API トークン）で渡す。
  backend "s3" {
    bucket = "zoo-terraform-state"
    key    = "r2/terraform.tfstate"
    region = "auto"
    endpoints = {
      s3 = "https://c9918f2caa050736a07867e6f968c0a2.r2.cloudflarestorage.com"
    }
    use_path_style              = true
    use_lockfile                = true
    skip_credentials_validation = true
    skip_region_validation      = true
    skip_requesting_account_id  = true
    skip_metadata_api_check     = true
    skip_s3_checksum            = true
  }

  # state と plan ファイルを暗号化する。passphrase は state-passphrase.sops.yaml を sops で復号して
  # TF_VAR_state_passphrase で渡す（README.md 参照）。
  encryption {
    key_provider "pbkdf2" "state" {
      passphrase = var.state_passphrase
      # state に保存する鍵メタデータの名前。passphrase を入れ替えるときに番号を上げる（README.md 参照）。
      encrypted_metadata_alias = "zoo2-v1"
    }

    method "aes_gcm" "state" {
      keys = key_provider.pbkdf2.state
    }

    state {
      method   = method.aes_gcm.state
      enforced = true
    }

    plan {
      method   = method.aes_gcm.state
      enforced = true
    }
  }
}
