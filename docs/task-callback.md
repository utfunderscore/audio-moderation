# External task callback

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

## Workflow integration prerequisite

The current audio-processing state machine ends after `ConvertAudio`; it has no
external dispatcher or callback task. Before this endpoint can resume an
execution, a dispatcher must receive the token and the state machine must use a
callback task, for example:

```json
"DispatchExternalTask": {
  "Type": "Task",
  "Resource": "arn:aws:states:::lambda:invoke.waitForTaskToken",
  "TimeoutSeconds": 3600,
  "Parameters": {
    "FunctionName": "<external-dispatcher-lambda-arn>",
    "Payload": {
      "input.$": "$",
      "taskToken.$": "$$.Task.Token"
    }
  },
  "End": true
}
```

`ConvertAudio` should transition to that state only once the dispatcher exists
and can securely hand the token to the external system. The callback success
`result` becomes the state output; the token must not be included in it.
