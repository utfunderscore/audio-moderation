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
The state machine then passes the returned transcription and stitched audio URI
to `moderation-caller` with a new task token. The moderation service returns that
second token after it has finished.
Before making an external request, its caller stores a SHA-256 digest of the
token against the exact pipeline step. The callback resolves that digest
transactionally and rejects unknown tokens, a job ID belonging to another task,
and use of a transcription token for moderation (or the reverse). Tokens are
bearer credentials and are neither stored nor logged in raw form.
The callback success `transcriptionResult` resumes the callback task and is
retained only long enough to construct the moderation request. The token must
not be included in the callback result. A successful callback body looks like:

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

The workflow's successful business outcome requires both **ASR and moderation
to complete and persist their results**. Callback results are persisted before
Step Functions delivery, so an unavailable service can be retried with the
identical callback. Failed callbacks resolve their step from the token, persist
that step's diagnostics first, and finalize only after Step Functions accepts
the failure.

## Moderation callback contract

The callback endpoint also accepts a moderation success with the same
`type: "success"` outcome tag as ASR. `moderationResult.jobId` must be a
numeric task ID and match the task resolved from `taskToken`. A mismatch marks
the moderation step failed and fails the workflow. `moderationTaskId` is
persisted; an identical retry is accepted, while a different ID conflicts.
`scores` is an object with exactly these five numeric categories; each score
must be finite and in the inclusive range `0.0..=1.0`. Missing or additional
categories are rejected.

```json
{
  "taskToken": "<task-token>",
  "outcome": {
    "type": "success",
    "moderationResult": {
      "jobId": "42",
      "moderationTaskId": "moderation_0123456789abcdef0123456789abcdef",
      "scores": {
        "sexual": 0.02,
        "hate_or_discrimination": 0.15,
        "harassment_or_abuse": 0.08,
        "violence_or_threats": 0.01,
        "asking_for_pii": 0.42
      }
    }
  }
}
```

Moderation results are persisted idempotently before `SendTaskSuccess`, which
receives only the nested `scores` object. A retry with different scores returns
a conflict without sending the task callback. `moderation-caller` records the
callback token before posting to `${MODAL_ENDPOINT_URL%/}/moderation/`, records
the returned external task ID, and makes request failures terminal. While
moderation is processing, successful finalization is rejected; once it
completes, successful finalization is accepted.
Waiter timeouts, aborted executions, and other terminal workflow statuses are
reconciled from Step Functions execution events.

`AWS_PROFILE=admin ./deployment-integration.sh test task-callback` is currently
unsupported. Its legacy isolated state machine has a real token but cannot
create the corresponding persisted callback attempt before that token is
generated. Do not seed a placeholder token or weaken callback correlation to
make that harness pass; use the production end-to-end workflow instead.
