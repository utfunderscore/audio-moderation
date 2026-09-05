resource "aws_ecr_repository" "confirm_upload" {
  name                 = "${local.name_prefix}-confirm-upload"
  image_tag_mutability = "MUTABLE"
  # Permit retirement of the old upload-complete repository during this rename.
  force_delete = true

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "confirm_upload_lambda_pull" {
  repository = aws_ecr_repository.confirm_upload.name
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

resource "aws_ecr_lifecycle_policy" "confirm_upload" {
  repository = aws_ecr_repository.confirm_upload.name
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

resource "aws_iam_role" "confirm_upload" {
  name = "${local.name_prefix}-confirm-upload"
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

resource "aws_iam_role_policy_attachment" "confirm_upload_logs" {
  role       = aws_iam_role.confirm_upload.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "confirm_upload_source_object" {
  name = "${local.name_prefix}-confirm-upload-source-object"
  role = aws_iam_role.confirm_upload.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "s3:GetObject"
      Resource = "${aws_s3_bucket.uploads.arn}/reviews/*/source"
    }]
  })
}

resource "aws_iam_role_policy" "confirm_upload_database_parameter" {
  name = "${local.name_prefix}-confirm-upload-database-parameter"
  role = aws_iam_role.confirm_upload.id
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

resource "aws_cloudwatch_log_group" "confirm_upload" {
  name              = "/aws/lambda/${local.name_prefix}-confirm-upload"
  retention_in_days = 7
}

resource "aws_lambda_function" "confirm_upload" {
  function_name = "${local.name_prefix}-confirm-upload"
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.confirm_upload.repository_url}:${var.confirm_upload_image_tag}"
  role          = aws_iam_role.confirm_upload.arn
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
    aws_ecr_repository_policy.confirm_upload_lambda_pull,
    aws_iam_role_policy_attachment.confirm_upload_logs,
    aws_iam_role_policy.confirm_upload_source_object,
    aws_iam_role_policy.confirm_upload_database_parameter,
    aws_cloudwatch_log_group.confirm_upload,
  ]
}

resource "aws_lambda_permission" "confirm_upload_s3" {
  statement_id   = "AllowS3Invoke"
  action         = "lambda:InvokeFunction"
  function_name  = aws_lambda_function.confirm_upload.function_name
  principal      = "s3.amazonaws.com"
  source_arn     = aws_s3_bucket.uploads.arn
  source_account = data.aws_caller_identity.current.account_id
}

resource "aws_s3_bucket_notification" "confirm_upload" {
  bucket = aws_s3_bucket.uploads.id

  lambda_function {
    lambda_function_arn = aws_lambda_function.confirm_upload.arn
    events              = ["s3:ObjectCreated:*"]
    filter_prefix       = "reviews/"
    filter_suffix       = "/source"
  }

  depends_on = [aws_lambda_permission.confirm_upload_s3]
}
