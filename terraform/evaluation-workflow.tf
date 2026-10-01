# Conversion -> transcription -> moderation orchestration and terminal-event
# reconciliation. The individual Lambda resources live in their own files.
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

output "audio_processing_state_machine_arn" {
  value = aws_sfn_state_machine.audio_processing.arn
}
