# Shared Cloudflare zone and certificate validation for both APIs. Custom
# domains, API mappings, and proxied DNS records live with the owning API.
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

locals {
  cloudflare_api_domains = var.enable_cloudflare_proxy ? {
    public      = var.public_api_domain_name
    task_events = var.task_events_domain_name
  } : {}
}

data "cloudflare_zone" "public" {
  # The Turnstile widget uses this account even when the API DNS proxy is off.
  count = 1

  filter = {
    name = var.cloudflare_zone_name
  }
}

resource "aws_acm_certificate" "public_apis" {
  for_each = local.cloudflare_api_domains

  domain_name       = each.value
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true

    precondition {
      condition = (
        var.public_api_domain_name != var.task_events_domain_name &&
        endswith(var.public_api_domain_name, ".${var.cloudflare_zone_name}") &&
        endswith(var.task_events_domain_name, ".${var.cloudflare_zone_name}")
      )
      error_message = "Cloudflare API hostnames must be distinct subdomains of cloudflare_zone_name."
    }
  }
}

resource "cloudflare_dns_record" "public_api_certificate_validation" {
  for_each = local.cloudflare_api_domains

  zone_id = data.cloudflare_zone.public[0].id
  name    = trimsuffix(try(one(aws_acm_certificate.public_apis[each.key].domain_validation_options).resource_record_name, "_refresh-only.${each.value}"), ".")
  type    = try(one(aws_acm_certificate.public_apis[each.key].domain_validation_options).resource_record_type, "CNAME")
  content = trimsuffix(try(one(aws_acm_certificate.public_apis[each.key].domain_validation_options).resource_record_value, "refresh-only.invalid"), ".")
  ttl     = 60
  proxied = false
  comment = "ACM validation for SocialGuard public APIs"
}

resource "aws_acm_certificate_validation" "public_apis" {
  for_each = local.cloudflare_api_domains

  certificate_arn         = try(aws_acm_certificate.public_apis[each.key].arn, "")
  validation_record_fqdns = [try(one(aws_acm_certificate.public_apis[each.key].domain_validation_options).resource_record_name, "")]

  depends_on = [cloudflare_dns_record.public_api_certificate_validation]
}
