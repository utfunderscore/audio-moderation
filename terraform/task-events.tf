variable "task_events_image_tag" {
  description = "Immutable image tag for the task-events Lambda"
  type        = string
}

variable "task_events_domain_name" {
  description = "Cloudflare-proxied hostname for the task-events WebSocket API"
  type        = string
  default     = "events-guard.utf.lol"
}

locals {
  task_events_websocket_endpoint  = var.enable_cloudflare_proxy ? "wss://${var.task_events_domain_name}" : trimsuffix(aws_apigatewayv2_stage.pipeline_task_events.invoke_url, "/")
  task_events_management_endpoint = var.enable_cloudflare_proxy ? "https://${var.task_events_domain_name}" : replace(trimsuffix(aws_apigatewayv2_stage.pipeline_task_events.invoke_url, "/"), "wss://", "https://")
}

resource "aws_ecr_repository" "task_events" {
  name                 = "${local.name_prefix}-task-events"
  image_tag_mutability = "IMMUTABLE"

  image_scanning_configuration { scan_on_push = true }
}

resource "aws_ecr_repository_policy" "task_events_lambda_pull" {
  repository = aws_ecr_repository.task_events.name
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "lambda.amazonaws.com" }
      Action    = ["ecr:BatchGetImage", "ecr:GetDownloadUrlForLayer"]
    }]
  })
}

resource "aws_ecr_lifecycle_policy" "task_events" {
  repository = aws_ecr_repository.task_events.name
  policy = jsonencode({
    rules = [{
      rulePriority = 1
      description  = "Keep the three most recent images"
      selection    = { tagStatus = "any", countType = "imageCountMoreThan", countNumber = 3 }
      action       = { type = "expire" }
    }]
  })
}

resource "aws_iam_role" "task_events" {
  name = "${local.name_prefix}-task-events"
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "lambda.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}

resource "aws_iam_role_policy_attachment" "task_events_logs" {
  role       = aws_iam_role.task_events.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "task_events_database_parameter" {
  name = "${local.name_prefix}-task-events-database-parameter"
  role = aws_iam_role.task_events.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = "ssm:GetParameter", Resource = "arn:aws:ssm:${var.aws_region}:${data.aws_caller_identity.current.account_id}:parameter${var.database_parameter_name}" },
      { Effect = "Allow", Action = "kms:Decrypt", Resource = "arn:aws:kms:${var.aws_region}:${data.aws_caller_identity.current.account_id}:alias/aws/ssm" },
      { Effect = "Allow", Action = "execute-api:ManageConnections", Resource = "${aws_apigatewayv2_api.pipeline_task_events.execution_arn}/$default/POST/@connections/*" },
    ]
  })
}

resource "aws_cloudwatch_log_group" "task_events" {
  name              = "/aws/lambda/${local.name_prefix}-task-events"
  retention_in_days = 7
}

# The public API accepts WebSocket connections without authentication because
# clients receive event names only. The subscribe route consumes a one-use,
# task-scoped ticket; its database invariant permits one task stream per socket.
resource "aws_apigatewayv2_api" "pipeline_task_events" {
  name                         = "${local.name_prefix}-pipeline-task-events"
  protocol_type                = "WEBSOCKET"
  route_selection_expression   = "$request.body.action"
  disable_execute_api_endpoint = var.enable_cloudflare_proxy
}

resource "aws_lambda_function" "task_events" {
  function_name = "${local.name_prefix}-task-events"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.task_events.repository_url}:${var.task_events_image_tag}"
  role          = aws_iam_role.task_events.arn
  architectures = ["x86_64"]
  memory_size   = 256
  timeout       = 10

  environment {
    variables = {
      DATABASE_URL_PARAMETER          = var.database_parameter_name
      TASK_EVENTS_MANAGEMENT_ENDPOINT = local.task_events_management_endpoint
      RUST_LOG                        = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.task_events_lambda_pull,
    aws_iam_role_policy_attachment.task_events_logs,
    aws_iam_role_policy.task_events_database_parameter,
    aws_cloudwatch_log_group.task_events,
  ]
}

resource "aws_apigatewayv2_integration" "task_events" {
  api_id             = aws_apigatewayv2_api.pipeline_task_events.id
  integration_type   = "AWS_PROXY"
  integration_uri    = aws_lambda_function.task_events.invoke_arn
  integration_method = "POST"
}

resource "aws_apigatewayv2_route" "task_events_connect" {
  api_id             = aws_apigatewayv2_api.pipeline_task_events.id
  route_key          = "$connect"
  authorization_type = "NONE"
  target             = "integrations/${aws_apigatewayv2_integration.task_events.id}"
}

resource "aws_apigatewayv2_route" "task_events_subscribe" {
  api_id             = aws_apigatewayv2_api.pipeline_task_events.id
  route_key          = "subscribe"
  authorization_type = "NONE"
  target             = "integrations/${aws_apigatewayv2_integration.task_events.id}"
}

resource "aws_apigatewayv2_route" "task_events_disconnect" {
  api_id             = aws_apigatewayv2_api.pipeline_task_events.id
  route_key          = "$disconnect"
  authorization_type = "NONE"
  target             = "integrations/${aws_apigatewayv2_integration.task_events.id}"
}

resource "aws_lambda_permission" "task_events_api_gateway" {
  statement_id  = "AllowTaskEventsApiGatewayInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.task_events.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.pipeline_task_events.execution_arn}/*"
}

resource "aws_apigatewayv2_stage" "pipeline_task_events" {
  api_id      = aws_apigatewayv2_api.pipeline_task_events.id
  name        = "$default"
  auto_deploy = true

  default_route_settings {
    throttling_burst_limit = 10
    throttling_rate_limit  = 5
  }
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

# Lifecycle Lambdas publish event names directly through the WebSocket API's
# management endpoint. Clients recover any missed delivery from PostgreSQL.
resource "aws_iam_role_policy" "task_event_emission" {
  for_each = {
    audio_processing     = aws_iam_role.audio_processing.id
    confirm_upload       = aws_iam_role.confirm_upload.id
    task_callback        = aws_iam_role.task_callback.id
    transcription_caller = aws_iam_role.transcription_caller.id
    moderation_caller    = aws_iam_role.moderation_caller.id
  }

  name = "${local.name_prefix}-${replace(each.key, "_", "-")}-task-event-emission"
  role = each.value
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "execute-api:ManageConnections"
      Resource = "${aws_apigatewayv2_api.pipeline_task_events.execution_arn}/$default/POST/@connections/*"
    }]
  })
}

output "pipeline_task_events_websocket_endpoint" {
  value = local.task_events_websocket_endpoint
}

output "task_events_function_name" {
  value = aws_lambda_function.task_events.function_name
}
