data "aws_iam_openid_connect_provider" "modal" {
  arn = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:oidc-provider/oidc.modal.com"
}

resource "aws_iam_role" "modal_stitched_audio_reader" {
  name        = "${local.name_prefix}-modal-stitched-audio-reader"
  description = "Allows the configured Modal workspace to read stitched audio artifacts"

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
    ]
  })
}
