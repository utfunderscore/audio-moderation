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
