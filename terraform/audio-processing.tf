resource "aws_s3_bucket" "artifacts" {
  bucket = "${local.name_prefix}-artifacts-${random_id.artifacts_bucket_suffix.hex}"
}

resource "random_id" "artifacts_bucket_suffix" {
  byte_length = 4
}

resource "aws_s3_bucket_public_access_block" "artifacts" {
  bucket                  = aws_s3_bucket.artifacts.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_server_side_encryption_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  rule {
    id     = "expire-artifacts"
    status = "Enabled"

    filter {}

    expiration {
      days = var.artifact_retention_days
    }
  }
}

resource "aws_ecr_repository" "audio_processing" {
  name                 = "${local.name_prefix}-audio-processing"
  image_tag_mutability = "MUTABLE"

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
      RUST_LOG = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.audio_processing_lambda_pull,
    aws_iam_role_policy_attachment.audio_processing_logs,
    aws_iam_role_policy.audio_processing_objects,
    aws_cloudwatch_log_group.audio_processing,
  ]
}

resource "aws_iam_role" "audio_processing_state_machine" {
  name = "${local.name_prefix}-audio-processing-state-machine"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Service = "states.amazonaws.com"
      }
      Action = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy" "audio_processing_state_machine" {
  name = "${local.name_prefix}-audio-processing-state-machine"
  role = aws_iam_role.audio_processing_state_machine.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = "lambda:InvokeFunction"
        Resource = aws_lambda_function.audio_processing.arn
      },
      {
        Effect = "Allow"
        Action = [
          "logs:CreateLogDelivery",
          "logs:GetLogDelivery",
          "logs:UpdateLogDelivery",
          "logs:DeleteLogDelivery",
          "logs:ListLogDeliveries",
          "logs:PutResourcePolicy",
          "logs:DescribeResourcePolicies",
          "logs:DescribeLogGroups",
        ]
        Resource = "*"
      },
    ]
  })
}

resource "aws_cloudwatch_log_group" "audio_processing_state_machine" {
  name              = "/aws/vendedlogs/states/${local.name_prefix}-audio-processing"
  retention_in_days = 7
}

resource "aws_sfn_state_machine" "audio_processing" {
  name     = "${local.name_prefix}-audio-processing"
  role_arn = aws_iam_role.audio_processing_state_machine.arn
  type     = "STANDARD"
  definition = jsonencode({
    Comment = "Convert ordered audio files into one normalized WAV artifact."
    StartAt = "ConvertAudio"
    States = {
      ConvertAudio = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke"
        Parameters = {
          FunctionName = aws_lambda_function.audio_processing.arn
          "Payload.$"  = "$"
        }
        ResultSelector = {
          "jobId.$"         = "$.Payload.jobId"
          "stitchedS3Uri.$" = "$.Payload.stitchedS3Uri"
        }
        Retry = [{
          ErrorEquals = [
            "Lambda.ServiceException",
            "Lambda.AWSLambdaException",
            "Lambda.SdkClientException",
            "Lambda.TooManyRequestsException",
          ]
          IntervalSeconds = 2
          BackoffRate     = 2
          MaxAttempts     = 3
        }]
        TimeoutSeconds = 870
        End            = true
      }
    }
  })

  logging_configuration {
    level                  = "ALL"
    include_execution_data = false
    log_destination        = "${aws_cloudwatch_log_group.audio_processing_state_machine.arn}:*"
  }

  depends_on = [aws_iam_role_policy.audio_processing_state_machine]
}
