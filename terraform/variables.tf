# Inputs shared across components. Component-specific inputs live alongside
# their resources rather than in this file.
variable "aws_region" {
  type    = string
  default = "eu-west-2"
}

variable "project_name" {
  type    = string
  default = "audio-moderation"
}

variable "environment" {
  type    = string
  default = "dev"
}

variable "database_parameter_name" {
  type    = string
  default = "/audio-moderation/dev/database-url"
}

variable "modal_proxy_token_id_parameter_name" {
  description = "Secure SSM parameter containing the Modal proxy token ID"
  type        = string
  default     = "/audio-moderation/dev/modal-proxy-token-id"
}

variable "modal_proxy_token_secret_parameter_name" {
  description = "Secure SSM parameter containing the Modal proxy token secret"
  type        = string
  default     = "/audio-moderation/dev/modal-proxy-token-secret"
}

variable "tenant_id" {
  description = "Temporary demo tenant stamped on jobs until the API is authenticated"
  type        = string
  default     = "default"
}

variable "browser_allowed_origins" {
  description = "Browser origins allowed to call the public HTTP API and upload through presigned S3 URLs"
  type        = set(string)
  default = [
    "http://127.0.0.1:4173",
    "http://127.0.0.1:5173",
    "http://127.0.0.1:8787",
    "http://localhost:4173",
    "http://localhost:5173",
    "http://localhost:8787",
    # API Gateway supports protocol wildcards. This covers HTTPS-hosted demos,
    # including Tailscale Funnel, without allowing arbitrary insecure origins.
    "https://*",
  ]

  validation {
    condition = length(var.browser_allowed_origins) > 0 && alltrue([
      for origin in var.browser_allowed_origins :
      can(regex("^https?://[^/]+$", origin))
    ])
    error_message = "browser_allowed_origins must contain origins such as https://app.example.com, without paths or trailing slashes."
  }
}
