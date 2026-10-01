variable "modal_workspace_id" {
  description = "Modal workspace ID permitted to assume the stitched-audio reader role"
  type        = string
  default     = "ac-k4lbrkEynY351mickkxfRh"

  validation {
    condition     = can(regex("^ac-[A-Za-z0-9]+$", var.modal_workspace_id))
    error_message = "modal_workspace_id must be a Modal workspace ID such as ac-12345abcd."
  }
}

data "aws_iam_openid_connect_provider" "modal" {
  arn = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:oidc-provider/oidc.modal.com"
}

resource "aws_iam_role" "modal_stitched_audio_reader" {
  name        = "${local.name_prefix}-modal-stitched-audio-reader"
  description = "Allows the configured Modal workspace to read stitched audio artifacts and invoke task callbacks"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Federated = data.aws_iam_openid_connect_provider.modal.arn
      }
      Action = "sts:AssumeRoleWithWebIdentity"
      Condition = {
        StringEquals = {
          "oidc.modal.com:aud" = "oidc.modal.com"
        }
        StringLike = {
          "oidc.modal.com:sub" = "modal:workspace_id:${var.modal_workspace_id}:*"
        }
      }
    }]
  })
}

resource "aws_iam_role_policy" "modal_stitched_audio_reader" {
  name = "${local.name_prefix}-modal-stitched-audio-reader"
  role = aws_iam_role.modal_stitched_audio_reader.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid      = "LocateStitchedAudioBucket"
        Effect   = "Allow"
        Action   = "s3:GetBucketLocation"
        Resource = aws_s3_bucket.artifacts.arn
      },
      {
        Sid      = "ListStitchedAudio"
        Effect   = "Allow"
        Action   = "s3:ListBucket"
        Resource = aws_s3_bucket.artifacts.arn
        Condition = {
          StringLike = {
            "s3:prefix" = "evaluations/*"
          }
        }
      },
      {
        Sid      = "ReadStitchedAudio"
        Effect   = "Allow"
        Action   = "s3:GetObject"
        Resource = "${aws_s3_bucket.artifacts.arn}/evaluations/*"
      },
      {
        Sid      = "InvokeTaskCallback"
        Effect   = "Allow"
        Action   = "lambda:InvokeFunction"
        Resource = aws_lambda_function.task_callback.arn
      },
      {
        Sid      = "CallTaskCallbackRoute"
        Effect   = "Allow"
        Action   = "execute-api:Invoke"
        Resource = "${aws_apigatewayv2_api.public.execution_arn}/${aws_apigatewayv2_stage.default.name}/POST/callbacks/external-task"
      },
    ]
  })
}

output "modal_stitched_audio_reader_role_arn" {
  description = "Set as oidc_auth_role_arn on Modal's read-only CloudBucketMount"
  value       = aws_iam_role.modal_stitched_audio_reader.arn
}
