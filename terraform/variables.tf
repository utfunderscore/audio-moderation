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

variable "evaluation_access_secret_parameter_name" {
  description = "Secure SSM parameter containing the evaluation capability signing secret"
  type        = string
  default     = "/audio-moderation/dev/evaluation-access-secret"
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
    "http://localhost:4173",
    "http://localhost:5173",
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
