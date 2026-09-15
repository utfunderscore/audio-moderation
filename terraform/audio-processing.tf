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
      DATABASE_URL_PARAMETER = var.database_parameter_name
      RUST_LOG               = "info"
    }
  }

  depends_on = [
    aws_ecr_repository_policy.audio_processing_lambda_pull,
    aws_iam_role_policy_attachment.audio_processing_logs,
    aws_iam_role_policy.audio_processing_objects,
    aws_iam_role_policy.audio_processing_database_parameter,
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
        Effect = "Allow"
        Action = "lambda:InvokeFunction"
        Resource = [
          aws_lambda_function.audio_processing.arn,
          aws_lambda_function.moderation_caller.arn,
          aws_lambda_function.transcription_caller.arn,
        ]
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
    Comment = "Convert audio, transcribe it, and moderate the resulting speech."
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
        Next           = "RequestTranscription"
        Catch = [{
          ErrorEquals = ["States.ALL"]
          ResultPath  = "$.workflowError"
          Next        = "FinalizeFailure"
        }]
      }
      RequestTranscription = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke.waitForTaskToken"
        Parameters = {
          FunctionName = aws_lambda_function.transcription_caller.arn
          Payload = {
            "jobId.$"     = "$.jobId"
            "audioUri.$"  = "$.stitchedS3Uri"
            "taskToken.$" = "$$.Task.Token"
          }
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
        ResultPath     = "$.transcriptionResult"
        TimeoutSeconds = 3600
        Next           = "RequestModeration"
        Catch = [{
          ErrorEquals = ["States.ALL"]
          ResultPath  = "$.workflowError"
          Next        = "FinalizeFailure"
        }]
      }
      RequestModeration = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke.waitForTaskToken"
        Parameters = {
          FunctionName = aws_lambda_function.moderation_caller.arn
          Payload = {
            "jobId.$"         = "$.jobId"
            "audioUri.$"      = "$.stitchedS3Uri"
            "transcription.$" = "$.transcriptionResult.transcription"
            "taskToken.$"     = "$$.Task.Token"
          }
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
        ResultPath     = null
        TimeoutSeconds = 3600
        Next           = "FinalizeSuccess"
        Catch = [{
          ErrorEquals = ["States.ALL"]
          ResultPath  = "$.workflowError"
          Next        = "FinalizeFailure"
        }]
      }
      FinalizeSuccess = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke"
        Parameters = {
          FunctionName = aws_lambda_function.audio_processing.arn
          Payload = {
            "jobId.$" = "$.jobId"
            outcome   = "SUCCEEDED"
          }
        }
        End = true
      }
      FinalizeFailure = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke"
        Parameters = {
          FunctionName = aws_lambda_function.audio_processing.arn
          Payload = {
            "jobId.$" = "$.jobId"
            outcome   = "FAILED"
            "error.$" = "$.workflowError.Error"
            "cause.$" = "$.workflowError.Cause"
          }
        }
        End = true
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

# Step Functions cannot run a Catch handler after an operator stops an
# execution. Reconcile every terminal execution, including ABORTED, through
# the database-aware worker without exposing task tokens in logs or events.
resource "aws_cloudwatch_event_rule" "audio_processing_terminal" {
  name = "${local.name_prefix}-audio-processing-terminal"
  event_pattern = jsonencode({
    source        = ["aws.states"]
    "detail-type" = ["Step Functions Execution Status Change"]
    detail = {
      stateMachineArn = [aws_sfn_state_machine.audio_processing.arn]
      status          = ["SUCCEEDED", "FAILED", "TIMED_OUT", "ABORTED"]
    }
  })
}

resource "aws_cloudwatch_event_target" "audio_processing_terminal" {
  rule = aws_cloudwatch_event_rule.audio_processing_terminal.name
  arn  = aws_lambda_function.audio_processing.arn
}

resource "aws_lambda_permission" "audio_processing_terminal_event" {
  statement_id  = "AllowTerminalExecutionEvents"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.audio_processing.function_name
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.audio_processing_terminal.arn
}
