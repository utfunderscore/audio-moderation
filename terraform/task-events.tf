# The public API accepts WebSocket connections without authentication because
# clients receive event names only. Route integrations are deliberately added
# with the future direct-notification Lambda rather than pointing at a placeholder.
resource "aws_apigatewayv2_api" "pipeline_task_events" {
  name                       = "${local.name_prefix}-pipeline-task-events"
  protocol_type              = "WEBSOCKET"
  route_selection_expression = "$request.body.action"
}

resource "aws_apigatewayv2_stage" "pipeline_task_events" {
  api_id      = aws_apigatewayv2_api.pipeline_task_events.id
  name        = "$default"
  auto_deploy = true

  default_route_settings {
    throttling_burst_limit = 10
    throttling_rate_limit  = 5
  }
}
