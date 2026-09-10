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
  default     = "latest"
}

variable "confirm_upload_image_tag" {
  description = "Immutable ECR image tag pushed before applying the Lambda configuration"
  type        = string
  default     = "latest"
}

variable "audio_processing_image_tag" {
  description = "Immutable ECR image tag pushed before applying the audio-processing Lambda configuration"
  type        = string
  default     = "latest"
}

variable "start_evaluation_image_tag" {
  description = "Immutable ECR image tag pushed before applying the start-evaluation Lambda configuration"
  type        = string
  default     = "latest"
}

variable "task_callback_image_tag" {
  description = "Immutable ECR image tag pushed before applying the task-callback Lambda configuration"
  type        = string
  default     = "latest"
}

variable "transcription_caller_image_tag" {
  description = "Immutable ECR image tag pushed before applying the transcription-caller Lambda configuration"
  type        = string
  default     = "latest"
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

variable "transcription_endpoint_url" {
  description = "Modal transcription API endpoint URL"
  type        = string
  default     = "https://utfunderscore--socialguard-transcription-api-submit-tran-0a77af.modal.run"
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
