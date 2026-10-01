# Testing an existing AWS deployment

From the repository root:

```sh
AWS_PROFILE=admin ./scripts/test-deployed.sh preflight task-events
AWS_PROFILE=admin ./scripts/test-deployed.sh test task-events
```

Use `--help` for options. `test` runs suite-specific preflight automatically.
These commands never apply infrastructure, publish images, or modify Modal.
Tests do create remote fixtures; obtain authorization for the selected remote
test before running it. Prefix all AWS CLI, Terraform, and deployment commands
with `AWS_PROFILE=admin`.

## Suites and prerequisites

All suites need the AWS CLI, Terraform, the `admin` profile, an existing
deployment, and current database schemas applied separately. Preflight does not
apply migrations, verify schema objects, decrypt secrets, or create fixtures.

| Suite | Additional prerequisites | Coverage |
| --- | --- | --- |
| `review-submit` | Cargo, database and Turnstile SecureStrings, fresh real `TURNSTILE_TEST_TOKEN` | Review submission, linked evaluation reads and tickets |
| `review-confirmation` | Same as review-submit, deployed WebSocket endpoint | Upload creates one task and dispatches evaluation; repeated PUT does not produce duplicate persisted dispatch effects during the observation window |
| `audio-conversion` | Cargo, database SecureString, readable nonempty `--audio-file` | Direct synchronous audio-processing invocation, output object, and persisted step result |
| `task-events` | Cargo, database SecureString, deployed WebSocket endpoint | Durable replay, live delivery, reconnect, and disconnect cleanup |

`task-callback`, `transcription-caller`, and `moderation-caller` are unsupported.
They require real Step Functions task tokens and compatible persisted task and
callback-attempt fixtures. Never bypass this with placeholder tokens or change
the model service contract just to manufacture a harness.

Review tests require a fresh real Turnstile response for the deployed widget
(`submit_review` action). Dummy tokens do not work against its managed secret.
Never print or commit tokens, database URLs, or decrypted secrets. The runner
checks token presence, not freshness or validity.

## Environment resolution

The runner reads required Terraform outputs from local `terraform/` state.
`AUDIO_MODERATION_API_ENDPOINT` and `AUDIO_MODERATION_TASK_EVENTS_ENDPOINT`
override endpoint outputs. The WebSocket URL must use `wss://`.

Database-backed test execution resolves `AUDIO_MODERATION_TENANT_ID` from the
deployed tenant unless already set and decrypts the configured database SSM
parameter only when `DATABASE_URL` is not already supplied. `--tenant-id` (or
`TENANT_ID`) controls the audio-conversion fixture's tenant. Cargo runs from
`backend/` so its offline SQLx configuration is loaded; only the named ignored
deployed test runs.

## Fixture lifetime and cleanup

- **review-submit:** the existing Rust test leaves its submitted review,
  evaluation, task, and ticket records in place; the runner does not clean them
  up or report a fixture manifest. Identify the exact records from the authorized
  run before manual cleanup. No source audio is uploaded by this suite.
- **review-confirmation:** verifies dispatch, not transcription/moderation
  completion or terminal review status. Do not delete fixtures while dispatched
  asynchronous work may still use them. The existing Rust test leaves these
  fixtures in place without a cleanup manifest. Identify the exact review,
  task, execution, and source/artifact objects from the authorized run, and wait
  for the workflow to terminate before removing them.
- **task-events:** the Rust test deletes successful fixtures. Failed fixtures
  retain their task ID for diagnosis; clean them up only after test activity ends.
- **audio-conversion:** the Rust test creates a task through `PipelineTaskStore`
  and removes its exact input/output S3 objects and task after the synchronous
  invocation, including ordinary failures. Cleanup failures identify the exact
  remaining fixture without printing the database URL.

Never perform broad bucket or database cleanup. Remove only identified fixtures
from the authorized test run, after asynchronous work has stopped.

## Implementation boundary and legacy command

`environment.sh` resolves test configuration, outputs, and secrets; `preflight.sh`
provides test checks. Only the selected file under `suites/` is sourced. Fixtures
and cleanup belong to that suite, not a global registry. Shared helpers under
`scripts/lib/` contain only target identity, parameter metadata validation, and
deployment-output reads.

The root `deployment-integration.sh` remains a compatibility dispatcher. Its
`preflight`, `deploy`, and `test` commands route to the scoped runners. `all`
first validates the suite, then deploys all eight Lambdas, then tests. Only that
initial combined check may defer a missing WebSocket endpoint; the test after
deployment always validates it. Prefer the scoped runners for normal work.
