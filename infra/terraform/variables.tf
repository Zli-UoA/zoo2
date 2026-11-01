variable "cloudflare_api_token" {
  type        = string
  description = "Cloudflare API Token with R2 permissions (Account.R2 Storage:Edit)"
  sensitive   = true
}

variable "cloudflare_account_id" {
  type        = string
  description = "Cloudflare Account ID"
}

variable "r2_location" {
  type        = string
  description = "R2 Bucket Location Hint (APAC, WNAM, ENAM, WEUR, EEUR)"
  default     = "apac"
}

variable "prod_bucket_name" {
  type        = string
  description = "Production R2 Bucket Name"
  default     = "zoo-attachments-prod"
}

variable "stg_bucket_name" {
  type        = string
  description = "Staging / Preview R2 Bucket Name"
  default     = "zoo-attachments-stg"
}

variable "prod_cors_origins" {
  type        = list(string)
  description = "Allowed origins for production R2 bucket CORS (public host of zoo2)"
  default     = ["https://zli.chat"]
}

variable "stg_cors_origins" {
  type        = list(string)
  description = "Allowed origins for staging / preview R2 bucket CORS (preview environments are https://zoo2-pr-<PR number>.zli.chat)"
  default     = ["https://zoo2-pr-*.zli.chat"]
}

variable "state_passphrase" {
  type        = string
  description = "Passphrase for OpenTofu state and plan encryption (decrypt state-passphrase.sops.yaml with sops)"
  sensitive   = true
}
