resource "aws_ecr_repository" "transcription_caller" {
  name                 = "${local.name_prefix}-transcription-caller"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "transcription_caller_lambda_pull" {
  repository = aws_ecr_repository.transcription_caller.name
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

resource "aws_ecr_lifecycle_policy" "transcription_caller" {
  repository = aws_ecr_repository.transcription_caller.name
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

resource "aws_iam_role" "transcription_caller" {
  name = "${local.name_prefix}-transcription-caller"
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

resource "aws_iam_role_policy_attachment" "transcription_caller_logs" {
  role       = aws_iam_role.transcription_caller.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "transcription_caller_modal_proxy_tokens" {
  name = "${local.name_prefix}-transcription-caller-modal-proxy-tokens"
  role = aws_iam_role.transcription_caller.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = "ssm:GetParameter"
        Resource = [
          "arn:aws:ssm:${var.aws_region}:${data.aws_caller_identity.current.account_id}:parameter${var.modal_proxy_token_id_parameter_name}",
          "arn:aws:ssm:${var.aws_region}:${data.aws_caller_identity.current.account_id}:parameter${var.modal_proxy_token_secret_parameter_name}",
        ]
      },
      {
        Effect   = "Allow"
        Action   = "kms:Decrypt"
        Resource = "arn:aws:kms:${var.aws_region}:${data.aws_caller_identity.current.account_id}:alias/aws/ssm"
      },
    ]
  })
}

resource "aws_iam_role_policy" "transcription_caller_database_parameter" {
  name = "${local.name_prefix}-transcription-caller-database-parameter"
  role = aws_iam_role.transcription_caller.id
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

resource "aws_cloudwatch_log_group" "transcription_caller" {
  name              = "/aws/lambda/${local.name_prefix}-transcription-caller"
  retention_in_days = 7
}

resource "aws_lambda_function" "transcription_caller" {
  function_name = "${local.name_prefix}-transcription-caller"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.transcription_caller.repository_url}:${var.transcription_caller_image_tag}"
  role          = aws_iam_role.transcription_caller.arn
  architectures = ["x86_64"]
  memory_size   = 256
  timeout       = 30

  environment {
    variables = {
      MODAL_PROXY_TOKEN_ID_PARAMETER     = var.modal_proxy_token_id_parameter_name
      MODAL_PROXY_TOKEN_SECRET_PARAMETER = var.modal_proxy_token_secret_parameter_name
      TRANSCRIPTION_ENDPOINT_URL         = var.transcription_endpoint_url
      DATABASE_URL_PARAMETER             = var.database_parameter_name
      RUST_LOG                           = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.transcription_caller_lambda_pull,
    aws_iam_role_policy_attachment.transcription_caller_logs,
    aws_iam_role_policy.transcription_caller_modal_proxy_tokens,
    aws_iam_role_policy.transcription_caller_database_parameter,
    aws_cloudwatch_log_group.transcription_caller,
  ]
}
