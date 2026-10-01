variable "audio_processing_image_tag" {
  description = "Immutable ECR image tag pushed before applying the audio-processing Lambda configuration"
  type        = string
}

variable "audio_source_bucket_arns" {
  description = "Additional S3 bucket ARNs from which the audio-processing Lambda may read audio objects"
  type        = set(string)
  default     = []
}

resource "aws_ecr_repository" "audio_processing" {
  name                 = "${local.name_prefix}-audio-processing"
  image_tag_mutability = "IMMUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "audio_processing_lambda_pull" {
  repository = aws_ecr_repository.audio_processing.name
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

resource "aws_ecr_lifecycle_policy" "audio_processing" {
  repository = aws_ecr_repository.audio_processing.name
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

resource "aws_iam_role" "audio_processing" {
  name = "${local.name_prefix}-audio-processing"
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

resource "aws_iam_role_policy_attachment" "audio_processing_logs" {
  role       = aws_iam_role.audio_processing.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "audio_processing_objects" {
  name = "${local.name_prefix}-audio-processing-objects"
  role = aws_iam_role.audio_processing.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = "s3:GetObject"
        Resource = concat(
          ["${aws_s3_bucket.uploads.arn}/reviews/*"],
          [for bucket_arn in var.audio_source_bucket_arns : "${trimsuffix(bucket_arn, "/")}/*"],
        )
      },
      {
        Effect   = "Allow"
        Action   = "s3:PutObject"
        Resource = "${aws_s3_bucket.artifacts.arn}/evaluations/*"
      },
    ]
  })
}

resource "aws_iam_role_policy" "audio_processing_database_parameter" {
  name = "${local.name_prefix}-audio-processing-database-parameter"
  role = aws_iam_role.audio_processing.id
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

resource "aws_cloudwatch_log_group" "audio_processing" {
  name              = "/aws/lambda/${local.name_prefix}-audio-processing"
  retention_in_days = 7
}

resource "aws_lambda_function" "audio_processing" {
  function_name = "${local.name_prefix}-audio-processing"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.audio_processing.repository_url}:${var.audio_processing_image_tag}"
  role          = aws_iam_role.audio_processing.arn
  architectures = ["x86_64"]
  memory_size   = 3008
  timeout       = 840

  ephemeral_storage {
    size = 4096
  }

  environment {
    variables = {
      DATABASE_URL_PARAMETER          = var.database_parameter_name
      TASK_EVENTS_MANAGEMENT_ENDPOINT = local.task_events_management_endpoint
      RUST_LOG                        = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.audio_processing_lambda_pull,
    aws_iam_role_policy_attachment.audio_processing_logs,
    aws_iam_role_policy.audio_processing_objects,
    aws_iam_role_policy.audio_processing_database_parameter,
    aws_iam_role_policy.task_event_emission,
    aws_cloudwatch_log_group.audio_processing,
  ]
}

output "audio_processing_ecr_repository_url" {
  value = aws_ecr_repository.audio_processing.repository_url
}

output "audio_processing_function_name" {
  value = aws_lambda_function.audio_processing.function_name
}
