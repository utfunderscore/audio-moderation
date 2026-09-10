output "api_endpoint" {
  value = aws_apigatewayv2_stage.default.invoke_url
}

output "submit_audio_ecr_repository_url" {
  value = aws_ecr_repository.submit_audio.repository_url
}

output "submit_audio_function_name" {
  value = aws_lambda_function.submit_audio.function_name
}

output "confirm_upload_ecr_repository_url" {
  value = aws_ecr_repository.confirm_upload.repository_url
}

output "confirm_upload_function_name" {
  value = aws_lambda_function.confirm_upload.function_name
}

output "uploads_bucket_name" {
  value = aws_s3_bucket.uploads.bucket
}

output "artifacts_bucket_name" {
  value = aws_s3_bucket.artifacts.bucket
}

output "modal_stitched_audio_reader_role_arn" {
  description = "Set as oidc_auth_role_arn on Modal's read-only CloudBucketMount"
  value       = aws_iam_role.modal_stitched_audio_reader.arn
}

output "audio_processing_ecr_repository_url" {
  value = aws_ecr_repository.audio_processing.repository_url
}

output "audio_processing_function_name" {
  value = aws_lambda_function.audio_processing.function_name
}

output "start_evaluation_ecr_repository_url" {
  value = aws_ecr_repository.start_evaluation.repository_url
}

output "start_evaluation_function_name" {
  value = aws_lambda_function.start_evaluation.function_name
}

output "task_callback_ecr_repository_url" {
  value = aws_ecr_repository.task_callback.repository_url
}

output "task_callback_function_name" {
  value = aws_lambda_function.task_callback.function_name
}

output "transcription_caller_ecr_repository_url" {
  value = aws_ecr_repository.transcription_caller.repository_url
}

output "transcription_caller_function_name" {
  value = aws_lambda_function.transcription_caller.function_name
}

output "aws_region" {
  value = var.aws_region
}

output "database_parameter_name" {
  value = var.database_parameter_name
}

output "tenant_id" {
  value = var.tenant_id
}

output "audio_processing_state_machine_arn" {
  value = aws_sfn_state_machine.audio_processing.arn
}

output "resource_group_name" {
  value = aws_resourcegroups_group.application.name
}

output "resource_group_arn" {
  value = aws_resourcegroups_group.application.arn
}
