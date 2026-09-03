resource "aws_ecr_repository" "submit_audio" {
  name                 = "${local.name_prefix}-submit-audio"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_repository_policy" "submit_audio_lambda_pull" {
  repository = aws_ecr_repository.submit_audio.name
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

resource "aws_ecr_lifecycle_policy" "submit_audio" {
  repository = aws_ecr_repository.submit_audio.name
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

resource "aws_iam_role" "submit_audio" {
  name = "${local.name_prefix}-submit-audio"
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

resource "aws_iam_role_policy_attachment" "submit_audio_logs" {
  role       = aws_iam_role.submit_audio.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "submit_audio_upload" {
  name = "${local.name_prefix}-submit-audio-upload"
  role = aws_iam_role.submit_audio.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "s3:PutObject"
      Resource = "${aws_s3_bucket.uploads.arn}/reviews/*"
    }]
  })
}

resource "aws_cloudwatch_log_group" "submit_audio" {
  name              = "/aws/lambda/${local.name_prefix}-submit-audio"
  retention_in_days = 7
}

resource "terraform_data" "submit_audio_image" {
  triggers_replace = [
    filesha256("${path.module}/../Cargo.lock"),
    filesha256("${path.module}/../Cargo.toml"),
    filesha256("${path.module}/../crates/submit-audio-lambda/Cargo.toml"),
    filesha256("${path.module}/../crates/submit-audio-lambda/Dockerfile"),
    filesha256("${path.module}/../crates/submit-audio-lambda/build.rs"),
    filesha256("${path.module}/../crates/submit-audio-lambda/src/main.rs"),
    filesha256("${path.module}/../crates/submit-audio-lambda/src/proto.rs"),
    filesha256("${path.module}/../proto/audio/review/v1/audio_review.proto"),
    var.image_tag,
  ]

  provisioner "local-exec" {
    working_dir = path.module
    command     = <<-EOT
      aws ecr get-login-password --region ${var.aws_region} | docker login --username AWS --password-stdin ${data.aws_caller_identity.current.account_id}.dkr.ecr.${var.aws_region}.amazonaws.com
      docker build --platform linux/amd64 --file ../crates/submit-audio-lambda/Dockerfile --tag ${aws_ecr_repository.submit_audio.repository_url}:${var.image_tag} ..
      docker push ${aws_ecr_repository.submit_audio.repository_url}:${var.image_tag}
    EOT
  }

  depends_on = [aws_ecr_repository.submit_audio]
}

resource "aws_lambda_function" "submit_audio" {
  function_name                  = "${local.name_prefix}-submit-audio"
  package_type                   = "Image"
  image_uri                      = "${aws_ecr_repository.submit_audio.repository_url}:${var.image_tag}"
  role                           = aws_iam_role.submit_audio.arn
  architectures                  = ["x86_64"]
  memory_size                    = 256
  timeout                        = 10
  reserved_concurrent_executions = 2

  environment {
    variables = {
      DATABASE_URL = var.database_url
    }
  }

  depends_on = [
    aws_ecr_repository_policy.submit_audio_lambda_pull,
    aws_iam_role_policy_attachment.submit_audio_logs,
    aws_iam_role_policy.submit_audio_upload,
    aws_cloudwatch_log_group.submit_audio,
    terraform_data.submit_audio_image,
  ]
}

resource "aws_apigatewayv2_integration" "submit_audio" {
  api_id                 = aws_apigatewayv2_api.public.id
  integration_type       = "AWS_PROXY"
  integration_uri        = aws_lambda_function.submit_audio.invoke_arn
  integration_method     = "POST"
  payload_format_version = "2.0"
}

resource "aws_apigatewayv2_route" "submit_audio_connect_rpc" {
  api_id    = aws_apigatewayv2_api.public.id
  route_key = "ANY /{proxy+}"
  target    = "integrations/${aws_apigatewayv2_integration.submit_audio.id}"
}

resource "aws_lambda_permission" "submit_audio_api_gateway" {
  statement_id  = "AllowApiGatewayInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.submit_audio.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.public.execution_arn}/*/*"
}
