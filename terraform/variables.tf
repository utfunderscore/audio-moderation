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

variable "submit_audio_image_tag" {
  description = "Immutable ECR image tag pushed before applying the Lambda configuration"
  type        = string
}

variable "confirm_upload_image_tag" {
  description = "Immutable ECR image tag pushed before applying the Lambda configuration"
  type        = string
}

variable "audio_processing_image_tag" {
  description = "Immutable ECR image tag pushed before applying the audio-processing Lambda configuration"
  type        = string
}

variable "start_evaluation_image_tag" {
  description = "Immutable ECR image tag pushed before applying the start-evaluation Lambda configuration"
  type        = string
}

variable "task_callback_image_tag" {
  description = "Immutable ECR image tag pushed before applying the task-callback Lambda configuration"
  type        = string
}

variable "transcription_caller_image_tag" {
  description = "Immutable ECR image tag pushed before applying the transcription-caller Lambda configuration"
  type        = string
}

variable "moderation_caller_image_tag" {
  description = "Immutable ECR image tag pushed before applying the moderation-caller Lambda configuration"
  type        = string
}

variable "task_events_image_tag" {
  description = "Immutable image tag for the task-events Lambda"
  type        = string
}

variable "database_parameter_name" {
  type    = string
  default = "/audio-moderation/dev/database-url"
}

variable "turnstile_secret_key_parameter_name" {
  description = "SecureString SSM parameter containing the Turnstile Siteverify secret (not the public sitekey)"
  type        = string
  default     = "/audio-moderation/dev/turnstile-secret-key"

  validation {
    condition     = can(regex("^/[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$", var.turnstile_secret_key_parameter_name))
    error_message = "turnstile_secret_key_parameter_name must be an absolute SSM parameter path such as /project/environment/turnstile-secret-key."
  }
}

variable "turnstile_allowed_hostnames" {
  description = "Comma-separated exact frontend hostnames accepted from Turnstile Siteverify (no schemes, ports, paths, or wildcards)"
  type        = string

  validation {
    condition = alltrue([
      for hostname in split(",", var.turnstile_allowed_hostnames) :
      can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$", hostname))
    ])
    error_message = "turnstile_allowed_hostnames must be a nonempty comma-separated list of lowercase exact hostnames without spaces, schemes, ports, or wildcards."
  }
}

variable "turnstile_expected_action" {
  description = "Turnstile Siteverify action required for SubmitReview; use test only with isolated dummy-key integration environments"
  type        = string
  default     = "submit_review"

  validation {
    condition     = can(regex("^[A-Za-z0-9_-]{1,32}$", var.turnstile_expected_action))
    error_message = "turnstile_expected_action must be a nonempty Turnstile action (1-32 letters, digits, underscores, or hyphens)."
  }
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

variable "transcription_endpoint_url" {
  description = "Compatible external transcription API endpoint URL"
  type        = string
  default     = "https://utfunderscore-development--socialguard-transcription-serve-api.modal.run"
}

variable "modal_endpoint_url" {
  description = "Base URL for the Modal moderation API"
  type        = string
}

variable "tenant_id" {
  description = "Temporary demo tenant stamped on jobs until the API is authenticated"
  type        = string
  default     = "default"
}

variable "upload_retention_days" {
  type    = number
  default = 7
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

variable "enable_cloudflare_proxy" {
  description = "Create API Gateway custom domains and proxy them through Cloudflare"
  type        = bool
  default     = false
}

variable "cloudflare_zone_name" {
  description = "Cloudflare DNS zone that owns the public API hostnames"
  type        = string
  default     = "utf.lol"
}

variable "public_api_domain_name" {
  description = "Cloudflare-proxied hostname for the public HTTP API"
  type        = string
  default     = "api-guard.utf.lol"
}

variable "task_events_domain_name" {
  description = "Cloudflare-proxied hostname for the task-events WebSocket API"
  type        = string
  default     = "events-guard.utf.lol"
}

variable "audio_source_bucket_arns" {
  description = "Additional S3 bucket ARNs from which the audio-processing Lambda may read audio objects"
  type        = set(string)
  default     = []
}

variable "artifacts_retention_days" {
  description = "Number of days to retain generated evaluation artifacts"
  type        = number
  default     = 30
}

variable "modal_workspace_id" {
  description = "Modal workspace ID permitted to assume the stitched-audio reader role"
  type        = string
  default     = "ac-k4lbrkEynY351mickkxfRh"

  validation {
    condition     = can(regex("^ac-[A-Za-z0-9]+$", var.modal_workspace_id))
    error_message = "modal_workspace_id must be a Modal workspace ID such as ac-12345abcd."
  }
}

variable "enable_test_resources" {
  description = "Create deployed integration-test-only infrastructure"
  type        = bool
  default     = false
}
