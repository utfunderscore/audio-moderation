resource "aws_ecr_repository" "task_callback" {
  name                 = "${local.name_prefix}-task-callback"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "task_callback_lambda_pull" {
  repository = aws_ecr_repository.task_callback.name
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Service = "lambda.amazonaws.com"
      }
      Action = [
        "ecr:BatchGetImage",
        "ecr:GetDownloadUrlForLayer",
      ]
    }]
  })
}

resource "aws_ecr_lifecycle_policy" "task_callback" {
  repository = aws_ecr_repository.task_callback.name
  policy = jsonencode({
    rules = [{
      rulePriority = 1
      description  = "Keep the three most recent images"
      selection = {
        tagStatus   = "any"
        countType   = "imageCountMoreThan"
        countNumber = 3
      }
      action = {
        type = "expire"
      }
    }]
  })
}

resource "aws_iam_role" "task_callback" {
  name = "${local.name_prefix}-task-callback"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Service = "lambda.amazonaws.com"
      }
      Action = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "task_callback_logs" {
  role       = aws_iam_role.task_callback.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

# Step Functions task-token APIs do not support resource-level permissions.
resource "aws_iam_role_policy" "task_callback_state_machine" {
  name = "${local.name_prefix}-task-callback-state-machine"
  role = aws_iam_role.task_callback.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Action = [
        "states:SendTaskSuccess",
        "states:SendTaskFailure",
      ]
      Resource = "*"
    }]
  })
}

resource "aws_cloudwatch_log_group" "task_callback" {
  name              = "/aws/lambda/${local.name_prefix}-task-callback"
  retention_in_days = 7
}

resource "aws_lambda_function" "task_callback" {
  function_name = "${local.name_prefix}-task-callback"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.task_callback.repository_url}:${var.task_callback_image_tag}"
  role          = aws_iam_role.task_callback.arn
  architectures = ["x86_64"]
  memory_size   = 256
  timeout       = 10

  environment {
    variables = {
      RUST_LOG = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.task_callback_lambda_pull,
    aws_iam_role_policy_attachment.task_callback_logs,
    aws_iam_role_policy.task_callback_state_machine,
    aws_cloudwatch_log_group.task_callback,
  ]
}

resource "aws_apigatewayv2_integration" "task_callback" {
  api_id                 = aws_apigatewayv2_api.public.id
  integration_type       = "AWS_PROXY"
  integration_uri        = aws_lambda_function.task_callback.invoke_arn
  integration_method     = "POST"
  payload_format_version = "2.0"
}

resource "aws_apigatewayv2_route" "task_callback" {
  api_id             = aws_apigatewayv2_api.public.id
  route_key          = "POST /callbacks/external-task"
  authorization_type = "AWS_IAM"
  target             = "integrations/${aws_apigatewayv2_integration.task_callback.id}"
}

resource "aws_lambda_permission" "task_callback_api_gateway" {
  statement_id  = "AllowApiGatewayInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.task_callback.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.public.execution_arn}/$default/POST/callbacks/external-task"
}
