# Lifecycle Lambdas publish event names directly through the WebSocket API's
# management endpoint. Clients recover any missed delivery from PostgreSQL.
resource "aws_iam_role_policy" "task_event_emission" {
  for_each = {
    audio_processing     = aws_iam_role.audio_processing.id
    confirm_upload       = aws_iam_role.confirm_upload.id
    start_evaluation     = aws_iam_role.start_evaluation.id
    task_callback        = aws_iam_role.task_callback.id
    transcription_caller = aws_iam_role.transcription_caller.id
    moderation_caller    = aws_iam_role.moderation_caller.id
  }

  name = "${local.name_prefix}-${replace(each.key, "_", "-")}-task-event-emission"
  role = each.value
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "execute-api:ManageConnections"
      Resource = "${aws_apigatewayv2_api.pipeline_task_events.execution_arn}/$default/POST/@connections/*"
    }]
  })
}
