# External task callback

## Direct invocation from Modal

The Modal OIDC role (`modal_stitched_audio_reader`) grants
`lambda:InvokeFunction` on the task-callback Lambda. Modal can use temporary AWS
credentials obtained by assuming that role with its OIDC token to invoke the
function directly through boto3. The function name is available in the Terraform
output `task_callback_function_name`.

The current Lambda entry point uses `lambda_http` and expects an API Gateway
event envelope, with the callback JSON serialized in its `body` field. Direct
invocation does not yet accept a plain callback JSON object. Callers should check
both Lambda invocation errors and the HTTP-style response returned by the handler.

## HTTP invocation

`POST /callbacks/external-task` is protected by API Gateway HTTP API `AWS_IAM`
authorization. Callers must send a SigV4-signed request and have an
`execute-api:Invoke` policy scoped to this route, for example:

```json
{
  "Effect": "Allow",
  "Action": "execute-api:Invoke",
  "Resource": "arn:aws:execute-api:<region>:<account>:<api-id>/$default/POST/callbacks/external-task"
}
```

The Lambda does not verify SigV4 itself. Its execution role is limited to
`states:SendTaskSuccess` and `states:SendTaskFailure` (these task-token APIs
require `Resource: "*"`), in addition to basic Lambda logging.

## Workflow integration

The audio-processing state machine calls `transcription-caller` after
`ConvertAudio` with a Step Functions task token. The external transcription
service must return that token to this callback endpoint after it has finished.
The callback success `transcriptionResult` resumes the callback task, but the
production state machine uses `ResultPath = null` and discards that result; the
terminal execution does not retain it. The token must not be included in the
callback result. A successful callback body looks like:

```json
{
  "taskToken": "<task-token>",
  "outcome": {
    "type": "success",
    "transcriptionResult": {
      "jobId": "42",
      "asrTaskId": "fc-123",
      "transcription": "spoken words..."
    }
  }
}
```

`AWS_PROFILE=admin ./deployment-integration.sh test task-callback` starts an
isolated test-only Step Functions state machine. It directly invokes the
deployed callback Lambda with a synthetic API Gateway v2 event envelope and a
real task token, then waits for that test execution to succeed. It does not
traverse API Gateway or modify the production state machine or Lambda
configuration.
