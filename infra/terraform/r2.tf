provider "cloudflare" {
  api_token = var.cloudflare_api_token
}

# -----------------------------------------------------------------------------
# 1. 本番環境 R2 バケット (zoo-attachments-prod)
# -----------------------------------------------------------------------------

resource "cloudflare_r2_bucket" "prod" {
  account_id = var.cloudflare_account_id
  name       = var.prod_bucket_name
  location   = var.r2_location
}

resource "cloudflare_r2_bucket_cors" "prod" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.prod.name

  rules = [
    {
      id = "allow-prod-frontend"
      allowed = {
        origins = var.prod_cors_origins
        methods = ["GET", "PUT", "HEAD"]
        headers = ["Content-Type", "Content-Length", "If-None-Match", "x-amz-*"]
      }
      expose_headers  = ["ETag"]
      max_age_seconds = 3600
    }
  ]
}

resource "cloudflare_r2_bucket_lifecycle" "prod" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.prod.name

  rules = [
    {
      id      = "abort-incomplete-multipart-uploads"
      enabled = true
      conditions = {
        prefix = ""
      }
      abort_multipart_uploads_transition = {
        condition = {
          max_age = 604800 # 7日間放置されたマルチパートアップロードを破棄
          type    = "Age"
        }
      }
    }
  ]
}

# -----------------------------------------------------------------------------
# 2. ステージング / Preview 環境 R2 バケット (zoo-attachments-stg)
# -----------------------------------------------------------------------------

resource "cloudflare_r2_bucket" "stg" {
  account_id = var.cloudflare_account_id
  name       = var.stg_bucket_name
  location   = var.r2_location
}

resource "cloudflare_r2_bucket_cors" "stg" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.stg.name

  rules = [
    {
      id = "allow-stg-and-preview-frontend"
      allowed = {
        origins = var.stg_cors_origins
        methods = ["GET", "PUT", "HEAD"]
        headers = ["Content-Type", "Content-Length", "If-None-Match", "x-amz-*"]
      }
      expose_headers  = ["ETag"]
      max_age_seconds = 3600
    }
  ]
}

resource "cloudflare_r2_bucket_lifecycle" "stg" {
  account_id  = var.cloudflare_account_id
  bucket_name = cloudflare_r2_bucket.stg.name

  rules = [
    {
      id      = "abort-incomplete-multipart-uploads"
      enabled = true
      conditions = {
        prefix = ""
      }
      abort_multipart_uploads_transition = {
        condition = {
          max_age = 604800 # 7日間放置されたマルチパートアップロードを破棄
          type    = "Age"
        }
      }
    }
  ]
}
