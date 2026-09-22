locals {
  public_api_endpoint             = var.enable_cloudflare_proxy ? "https://${var.public_api_domain_name}" : trimsuffix(aws_apigatewayv2_stage.default.invoke_url, "/")
  task_events_websocket_endpoint  = var.enable_cloudflare_proxy ? "wss://${var.task_events_domain_name}" : trimsuffix(aws_apigatewayv2_stage.pipeline_task_events.invoke_url, "/")
  task_events_management_endpoint = var.enable_cloudflare_proxy ? "https://${var.task_events_domain_name}" : replace(trimsuffix(aws_apigatewayv2_stage.pipeline_task_events.invoke_url, "/"), "wss://", "https://")
  cloudflare_api_domains = var.enable_cloudflare_proxy ? {
    public      = var.public_api_domain_name
    task_events = var.task_events_domain_name
  } : {}
}

data "cloudflare_zone" "public" {
  count = var.enable_cloudflare_proxy ? 1 : 0

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

resource "aws_apigatewayv2_domain_name" "task_events" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  domain_name = var.task_events_domain_name

  domain_name_configuration {
    certificate_arn = aws_acm_certificate_validation.public_apis["task_events"].certificate_arn
    endpoint_type   = "REGIONAL"
    security_policy = "TLS_1_2"
  }
}

resource "aws_apigatewayv2_api_mapping" "task_events" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  api_id      = aws_apigatewayv2_api.pipeline_task_events.id
  domain_name = aws_apigatewayv2_domain_name.task_events[0].id
  stage       = aws_apigatewayv2_stage.pipeline_task_events.id
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

resource "cloudflare_dns_record" "task_events" {
  count = var.enable_cloudflare_proxy ? 1 : 0

  zone_id = data.cloudflare_zone.public[0].id
  name    = var.task_events_domain_name
  type    = "CNAME"
  content = aws_apigatewayv2_domain_name.task_events[0].domain_name_configuration[0].target_domain_name
  ttl     = 1
  proxied = true
  comment = "SocialGuard task-events WebSocket API"
}
