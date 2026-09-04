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
  type    = string
  default = "latest"
}

variable "database_parameter_name" {
  type    = string
  default = "/audio-moderation/dev/database-url"
}

variable "upload_retention_days" {
  type    = number
  default = 7
}
