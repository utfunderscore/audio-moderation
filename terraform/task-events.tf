# The public API accepts WebSocket connections without authentication because
# clients receive event names only.
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

resource "aws_apigatewayv2_api" "pipeline_task_events" {
  name                       = "${local.name_prefix}-pipeline-task-events"
  protocol_type              = "WEBSOCKET"
  route_selection_expression = "$request.body.action"
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
      TASK_EVENTS_MANAGEMENT_ENDPOINT = replace(aws_apigatewayv2_stage.pipeline_task_events.invoke_url, "wss://", "https://")
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
