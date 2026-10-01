# Shared HTTP API. Lambda integrations and routes belong to their Lambda files.
variable "public_api_domain_name" {
  description = "Cloudflare-proxied hostname for the public HTTP API"
  type        = string
  default     = "api-guard.utf.lol"
}

locals {
  public_api_endpoint = var.enable_cloudflare_proxy ? "https://${var.public_api_domain_name}" : trimsuffix(aws_apigatewayv2_stage.default.invoke_url, "/")
}

resource "aws_apigatewayv2_api" "public" {
  name                         = "${local.name_prefix}-api"
  protocol_type                = "HTTP"
  disable_execute_api_endpoint = var.enable_cloudflare_proxy

  cors_configuration {
    allow_headers = [
      "authorization",
      "connect-protocol-version",
      "connect-timeout-ms",
      "content-type",
      "idempotency-key",
    ]
    allow_methods = ["OPTIONS", "POST"]
    allow_origins = var.browser_allowed_origins
    max_age       = 3600
  }
}

resource "aws_apigatewayv2_stage" "default" {
  api_id      = aws_apigatewayv2_api.public.id
  name        = "$default"
  auto_deploy = true

  default_route_settings {
    throttling_burst_limit = 10
    throttling_rate_limit  = 5
  }
}

resource "aws_apigatewayv2_domain_name" "public" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  domain_name = var.public_api_domain_name

  domain_name_configuration {
    certificate_arn = aws_acm_certificate_validation.public_apis["public"].certificate_arn
    endpoint_type   = "REGIONAL"
    security_policy = "TLS_1_2"
  }
}

resource "aws_apigatewayv2_api_mapping" "public" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  api_id      = aws_apigatewayv2_api.public.id
  domain_name = aws_apigatewayv2_domain_name.public[0].id
  stage       = aws_apigatewayv2_stage.default.id
}

resource "cloudflare_dns_record" "public_api" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  zone_id = data.cloudflare_zone.public[0].id
  name    = var.public_api_domain_name
  type    = "CNAME"
  content = aws_apigatewayv2_domain_name.public[0].domain_name_configuration[0].target_domain_name
  ttl     = 1
  proxied = true
  comment = "SocialGuard public HTTP API"
}

output "api_endpoint" {
  value = local.public_api_endpoint
}
