resource "aws_ecr_repository" "start_evaluation" {
  name                 = "${local.name_prefix}-start-evaluation"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "start_evaluation_lambda_pull" {
  repository = aws_ecr_repository.start_evaluation.name
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

resource "aws_ecr_lifecycle_policy" "start_evaluation" {
  repository = aws_ecr_repository.start_evaluation.name
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

resource "aws_iam_role" "start_evaluation" {
  name = "${local.name_prefix}-start-evaluation"
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

resource "aws_iam_role_policy_attachment" "start_evaluation_logs" {
  role       = aws_iam_role.start_evaluation.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "start_evaluation_database_parameter" {
  name = "${local.name_prefix}-start-evaluation-database-parameter"
  role = aws_iam_role.start_evaluation.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = "ssm:GetParameter"
        Resource = "arn:aws:ssm:${var.aws_region}:${data.aws_caller_identity.current.account_id}:parameter${var.database_parameter_name}"
      },
      {
        Effect   = "Allow"
        Action   = "kms:Decrypt"
        Resource = "arn:aws:kms:${var.aws_region}:${data.aws_caller_identity.current.account_id}:alias/aws/ssm"
      },
    ]
  })
}

resource "aws_iam_role_policy" "start_evaluation_state_machine" {
  name = "${local.name_prefix}-start-evaluation-state-machine"
  role = aws_iam_role.start_evaluation.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = "states:StartExecution"
        Resource = aws_sfn_state_machine.audio_processing.arn
      },
      {
        Effect   = "Allow"
        Action   = "states:DescribeExecution"
        Resource = "${replace(aws_sfn_state_machine.audio_processing.arn, ":stateMachine:", ":execution:")}:*"
      },
    ]
  })
}

resource "aws_cloudwatch_log_group" "start_evaluation" {
  name              = "/aws/lambda/${local.name_prefix}-start-evaluation"
  retention_in_days = 7
}

resource "aws_lambda_function" "start_evaluation" {
  function_name = "${local.name_prefix}-start-evaluation"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.start_evaluation.repository_url}:${var.start_evaluation_image_tag}"
  role          = aws_iam_role.start_evaluation.arn
  architectures = ["x86_64"]
  memory_size   = 256
  timeout       = 10

  environment {
    variables = {
      ARTIFACTS_BUCKET_NAME  = aws_s3_bucket.artifacts.bucket
      DATABASE_URL_PARAMETER = var.database_parameter_name
      STATE_MACHINE_ARN      = aws_sfn_state_machine.audio_processing.arn
      TENANT_ID              = var.tenant_id
      RUST_LOG               = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.start_evaluation_lambda_pull,
    aws_iam_role_policy_attachment.start_evaluation_logs,
    aws_iam_role_policy.start_evaluation_database_parameter,
    aws_iam_role_policy.start_evaluation_state_machine,
    aws_cloudwatch_log_group.start_evaluation,
  ]
}

resource "aws_apigatewayv2_integration" "start_evaluation" {
  api_id                 = aws_apigatewayv2_api.public.id
  integration_type       = "AWS_PROXY"
  integration_uri        = aws_lambda_function.start_evaluation.invoke_arn
  integration_method     = "POST"
  payload_format_version = "2.0"
}

resource "aws_apigatewayv2_route" "start_evaluation_connect_rpc" {
  api_id    = aws_apigatewayv2_api.public.id
  route_key = "POST /audio.moderation.v1.AudioModerationService/StartEvaluation"
  target    = "integrations/${aws_apigatewayv2_integration.start_evaluation.id}"
}

resource "aws_lambda_permission" "start_evaluation_api_gateway" {
  statement_id  = "AllowApiGatewayInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.start_evaluation.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.public.execution_arn}/*/*"
}
