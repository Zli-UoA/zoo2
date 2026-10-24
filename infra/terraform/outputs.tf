output "prod_bucket_name" {
  description = "Production R2 bucket name"
  value       = cloudflare_r2_bucket.prod.name
}

output "prod_bucket_endpoint" {
  description = "Production S3-compatible R2 endpoint"
  value       = "https://${var.cloudflare_account_id}.r2.cloudflarestorage.com"
}

output "stg_bucket_name" {
  description = "Staging R2 bucket name"
  value       = cloudflare_r2_bucket.stg.name
}

output "stg_bucket_endpoint" {
  description = "Staging S3-compatible R2 endpoint"
  value       = "https://${var.cloudflare_account_id}.r2.cloudflarestorage.com"
}
