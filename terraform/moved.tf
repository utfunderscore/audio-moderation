moved {
  from = aws_ecr_repository.upload_complete
  to   = aws_ecr_repository.confirm_upload
}

moved {
  from = aws_ecr_repository_policy.upload_complete_lambda_pull
  to   = aws_ecr_repository_policy.confirm_upload_lambda_pull
}

moved {
  from = aws_ecr_lifecycle_policy.upload_complete
  to   = aws_ecr_lifecycle_policy.confirm_upload
}

moved {
  from = aws_iam_role.upload_complete
  to   = aws_iam_role.confirm_upload
}

moved {
  from = aws_iam_role_policy_attachment.upload_complete_logs
  to   = aws_iam_role_policy_attachment.confirm_upload_logs
}

moved {
  from = aws_iam_role_policy.upload_complete_source_object
  to   = aws_iam_role_policy.confirm_upload_source_object
}

moved {
  from = aws_iam_role_policy.upload_complete_database_parameter
  to   = aws_iam_role_policy.confirm_upload_database_parameter
}

moved {
  from = aws_cloudwatch_log_group.upload_complete
  to   = aws_cloudwatch_log_group.confirm_upload
}

moved {
  from = aws_lambda_function.upload_complete
  to   = aws_lambda_function.confirm_upload
}

moved {
  from = aws_lambda_permission.upload_complete_s3
  to   = aws_lambda_permission.confirm_upload_s3
}

moved {
  from = aws_s3_bucket_notification.upload_complete
  to   = aws_s3_bucket_notification.confirm_upload
}
