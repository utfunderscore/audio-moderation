resource "aws_ecr_repository" "upload_complete" {
  name                 = "${local.name_prefix}-upload-complete"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "upload_complete_lambda_pull" {
  repository = aws_ecr_repository.upload_complete.name
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

resource "aws_ecr_lifecycle_policy" "upload_complete" {
  repository = aws_ecr_repository.upload_complete.name
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

resource "aws_iam_role" "upload_complete" {
  name = "${local.name_prefix}-upload-complete"
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

resource "aws_iam_role_policy_attachment" "upload_complete_logs" {
  role       = aws_iam_role.upload_complete.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "upload_complete_source_object" {
  name = "${local.name_prefix}-upload-complete-source-object"
  role = aws_iam_role.upload_complete.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "s3:GetObject"
      Resource = "${aws_s3_bucket.uploads.arn}/reviews/*/source"
    }]
  })
}

resource "aws_iam_role_policy" "upload_complete_database_parameter" {
  name = "${local.name_prefix}-upload-complete-database-parameter"
  role = aws_iam_role.upload_complete.id
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

resource "aws_cloudwatch_log_group" "upload_complete" {
  name              = "/aws/lambda/${local.name_prefix}-upload-complete"
  retention_in_days = 7
}

resource "aws_lambda_function" "upload_complete" {
  function_name = "${local.name_prefix}-upload-complete"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.upload_complete.repository_url}:${var.upload_complete_image_tag}"
  role          = aws_iam_role.upload_complete.arn
  architectures = ["x86_64"]
  memory_size   = 256
  timeout       = 10

  environment {
    variables = {
      DATABASE_URL_PARAMETER = var.database_parameter_name
      UPLOADS_BUCKET_NAME    = aws_s3_bucket.uploads.bucket
      TENANT_ID              = var.tenant_id
      RUST_LOG               = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.upload_complete_lambda_pull,
    aws_iam_role_policy_attachment.upload_complete_logs,
    aws_iam_role_policy.upload_complete_source_object,
    aws_iam_role_policy.upload_complete_database_parameter,
    aws_cloudwatch_log_group.upload_complete,
  ]
}

resource "aws_lambda_permission" "upload_complete_s3" {
  statement_id   = "AllowS3Invoke"
  action         = "lambda:InvokeFunction"
  function_name  = aws_lambda_function.upload_complete.function_name
  principal      = "s3.amazonaws.com"
  source_arn     = aws_s3_bucket.uploads.arn
  source_account = data.aws_caller_identity.current.account_id
}

resource "aws_s3_bucket_notification" "upload_complete" {
  bucket = aws_s3_bucket.uploads.id

  lambda_function {
    lambda_function_arn = aws_lambda_function.upload_complete.arn
    events              = ["s3:ObjectCreated:*"]
    filter_prefix       = "reviews/"
    filter_suffix       = "/source"
  }

  depends_on = [aws_lambda_permission.upload_complete_s3]
}
