resource "aws_iam_role" "task_callback_test_state_machine" {
  count = var.enable_test_resources ? 1 : 0

  name = "${local.name_prefix}-task-callback-test-state-machine"
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

resource "aws_iam_role_policy" "task_callback_test_state_machine" {
  count = var.enable_test_resources ? 1 : 0

  name = "${local.name_prefix}-task-callback-test-state-machine"
  role = aws_iam_role.task_callback_test_state_machine[0].id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "lambda:InvokeFunction"
      Resource = aws_lambda_function.task_callback.arn
    }]
  })
}

# This test-only state machine supplies a real Step Functions task token to the
# deployed callback Lambda. It is isolated from the production workflow and
# never changes the production Lambda configuration.
resource "aws_sfn_state_machine" "task_callback_test" {
  count = var.enable_test_resources ? 1 : 0

  name     = "${local.name_prefix}-task-callback-test"
  role_arn = aws_iam_role.task_callback_test_state_machine[0].arn
  type     = "STANDARD"
  definition = jsonencode({
    StartAt = "CompleteViaCallback"
    States = {
      CompleteViaCallback = {
        Type     = "Task"
        Resource = "arn:aws:states:::lambda:invoke.waitForTaskToken"
        Parameters = {
          FunctionName = aws_lambda_function.task_callback.arn
          Payload = {
            version         = "2.0"
            routeKey        = "POST /callbacks/external-task"
            rawPath         = "/callbacks/external-task"
            rawQueryString  = ""
            isBase64Encoded = false
            headers         = { "content-type" = "application/json" }
            requestContext = {
              accountId    = "integration-test"
              apiId        = "integration-test"
              domainName   = "integration-test"
              domainPrefix = "integration-test"
              requestId    = "integration-test"
              routeKey     = "POST /callbacks/external-task"
              stage        = "$default"
              time         = "01/Jan/1970:00:00:00 +0000"
              timeEpoch    = 0
              http = {
                method    = "POST"
                path      = "/callbacks/external-task"
                protocol  = "HTTP/1.1"
                sourceIp  = "127.0.0.1"
                userAgent = "StepFunctionsIntegrationTest"
              }
            }
            "body.$" = "States.Format('\\{\"taskToken\":\"{}\",\"outcome\":\\{\"type\":\"success\",\"transcriptionResult\":\\{\"jobId\":\"integration-test\",\"asrTaskId\":\"integration-test\",\"transcription\":\"integration test\"\\}\\}\\}', $$.Task.Token)"
          }
        }
        TimeoutSeconds = 60
        End            = true
      }
    }
  })

  depends_on = [aws_iam_role_policy.task_callback_test_state_machine]
}
