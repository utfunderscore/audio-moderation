provider "aws" {
  region = var.aws_region

  default_tags {
    tags = local.resource_tags
  }
}

# Authentication is read from CLOUDFLARE_API_TOKEN, or from the legacy
# CLOUDFLARE_API_KEY plus CLOUDFLARE_EMAIL pair. Never place credentials in a
# Terraform variable because local state and plan files can persist variables.
provider "cloudflare" {}
