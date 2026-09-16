# Start evaluation Lambda

## Deployed integration tests

`tests/deployed.rs` is ignored by default. It calls the deployed Connect endpoint, reads the same PostgreSQL database used by the Lambda, and describes the resulting Step Functions executions. Run it through the repository-root `deployment-integration.sh` command rather than directly. It requires:

- a full deployment, including the `start-evaluation`, audio-processing, transcription-caller, moderation-caller, and task-callback Lambdas;
- applied Terraform for that deployment;
- AWS credentials for the local `admin` profile. When configuration is resolved automatically, they must allow Terraform state access and `ssm:GetParameter`/KMS decryption. Step Functions assertions also require `states:DescribeExecution`; `--audio-file` additionally requires `s3:PutObject` on the uploads bucket; and
- for dispatch and end-to-end evaluation, a regular, readable, nonempty local audio file passed with `--audio-file`.

The runner resolves omitted `AUDIO_MODERATION_API_ENDPOINT` and
`AUDIO_MODERATION_TENANT_ID` from Terraform outputs. It decrypts the configured
database SSM parameter without printing it. `terraform`, `aws`, and `jq` are
required when this discovery is needed and whenever `--audio-file` provisions
fixtures. Set `AUDIO_MODERATION_API_ENDPOINT`,
`AUDIO_MODERATION_TENANT_ID`, or `DATABASE_URL` explicitly to override this
automatic resolution.

For `evaluation-e2e`, the runner also resolves
`pipeline_task_events_websocket_endpoint` as
`AUDIO_MODERATION_TASK_EVENTS_ENDPOINT` and validates that it is a `wss://` URL.
After StartEvaluation returns, the test subscribes one socket to its task and
collects lifecycle events while it waits for the workflow. Event frames are plain
UTF-8 names and delivery is at-least-once, so the assertion requires all eight
successful lifecycle names while tolerating duplicate frames around the
replay/live boundary. It prints expected, observed, and persisted event history
alongside the existing workflow report.

Run the fixture-free ingress tests, dispatch tests, or full terminal-workflow test with the repository-root runner:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-ingress
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-dispatch \
  --audio-file ./sample_071.mp3
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-e2e \
  --audio-file ./sample_071.mp3 \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example \
```

The runner validates configuration without printing secret values and does not
deploy infrastructure for `test`. `evaluation-ingress` deliberately does not
need `AUDIO_MODERATION_TEST_AUDIO_S3_URIS`: it uses two syntactically valid,
distinct synthetic URIs while checking request validation, active leases,
terminal retry behavior, and idempotency conflicts without dispatching or
reading S3. `evaluation-e2e` and tests that dispatch the deployed workflow still
require real `AUDIO_MODERATION_TEST_AUDIO_S3_URIS`; it uploads the specified
local file twice below
`reviews/integration-tests/<run-id>/first.wav` and `second.wav`; neither key
ends in `/source`, so they cannot trigger the review confirmation notification.
It removes those fixtures only after the terminal workflow has succeeded. On a
failure it retains them and reports the prefix because asynchronous execution
may still need them.

`evaluation-dispatch` uploads fixtures in the same non-notifying location and
runs the seeded undispatched-task and failed-dispatch retry cases. Those tests
assert dispatch metadata but do not wait for the production workflows to
finish, so their fixtures are always retained and reported for asynchronous
processing and diagnosis.

The tests intentionally do not clean up their database rows or Step Functions executions so failures remain diagnosable. Running dispatch or end-to-end evaluation uses real AWS resources and has AWS cost; provide only real audio objects, never placeholder object URIs.

Fixtures are seeded through `PipelineTaskStore`; read-only SQL verifies persisted state without accidentally creating missing records. Unique idempotency keys isolate each run. The end-to-end suite waits for downstream audio processing, compatible external transcription and moderation, callback delivery, and terminal Step Functions success. Use a dedicated development environment with the current schema already applied; the tests do not run migrations. The checked-in socialguard-models application contract is not modified to resolve an endpoint mismatch; compatible endpoints are external prerequisites.

When the end-to-end workflow reaches a terminal state, the test prints a
structured `Processing result` JSON report. It includes the Step Functions
status, output, error and cause; the pipeline outcome; ordered input files;
produced artifact URIs; transcription text; moderation scores; external task
IDs; and each persisted step's status and collected error details. The report is
printed before either a successful return or a terminal-workflow test failure.
