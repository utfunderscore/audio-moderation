# Shared account identity, naming, and tags. Component resources live in their
# own files; see README.md for the ownership map.
data "aws_caller_identity" "current" {}

locals {
  name_prefix = "${var.project_name}-${var.environment}"
  resource_tags = {
    Project     = var.project_name
    Environment = var.environment
    ManagedBy   = "Terraform"
  }
}
