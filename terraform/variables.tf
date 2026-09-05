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

variable "database_parameter_name" {
  type    = string
  default = "/audio-moderation/dev/database-url"
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
