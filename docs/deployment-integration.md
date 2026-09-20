# Deployment Integration Runbook for Agents

Use `deployment-integration.sh` for every deployed AWS Lambda integration workflow in this repository. Do not recreate its steps with direct Docker, ECR, Lambda, or Terraform commands.

For a code-oriented walkthrough of the runner itself, see
[`deployment-integration-script.md`](deployment-integration-script.md).

## Safety Rules

- Always run the script from the repository root with `AWS_PROFILE=admin`.
- Run `preflight` before a deployment or deployed test.
- Do not deploy unless the user explicitly requested or approved a deployment. Deployment builds images, pushes to ECR, changes AWS resources, and can incur cost.
- Do not run deployments concurrently from separate worktrees. Terraform state is local and has no shared lock.
- Do not run direct `terraform apply` commands. Lambda image tags are required and the script coordinates ECR bootstrap, image publication, Terraform, and Lambda waiters.
- Do not use or introduce a `latest` Lambda image tag. A deployment must use one collision-checked immutable tag for all eight images.
- Test-only Terraform resources must remain gated by `enable_test_resources`, which defaults to `false`. The integration deployment script explicitly enables them.
- Never print decrypted SSM parameter values or database credentials.
- Do not deploy or modify Modal resources without explicit approval immediately before the Modal command.
- Do not assume the checked-in `socialguard-models` gateway is compatible with the Rust `TranscriptionRequest`. A compatible external endpoint must be verified separately.

Get command details without contacting AWS:

```sh
AWS_PROFILE=admin ./deployment-integration.sh --help
```

## Business Flow Boundaries

Review upload and evaluation are separate business flows:

```text
SubmitReview -> presigned S3 upload -> confirm-upload -> PENDING_PROCESSING
```

```text
StartEvaluation -> presigned evaluation upload -> confirm-upload -> Step Functions
  -> audio conversion -> transcription -> moderation -> callbacks
```

There is no review-to-evaluation integration. Do not describe `review-confirmation` as starting an evaluation, and do not construct a test that assumes it does.

## Commands

| Command | Purpose | Changes AWS? |
|---|---|---:|
| `preflight [suite]` | Validate tools, identity, configuration, and external prerequisites | Read-only AWS calls |
| `deploy` | Build and deploy all eight Lambda images and the complete Terraform environment | Yes |
| `test <suite>` | Run one suite against an existing deployment | Usually; tests create real data and some invoke AWS services |
| `all [suite]` | Deploy the complete environment, then run one suite | Yes |

Without a suite, `preflight` validates the complete deployment. Without a suite, `all` runs `evaluation-e2e` after deployment. Before `all` changes AWS resources, it validates the selected suite's local prerequisites (including its audio fixture and external endpoints); the task-events WebSocket endpoint alone is deferred because the deployment creates it. Deployment then performs its normal full preflight, and the suite validates the deployed WebSocket endpoint before connecting.

## Suites

| Suite | Coverage | Fixture | Status |
|---|---|---|---|
| `review-submit` | SubmitReview response and presigned upload details | None | Supported |
| `review-confirmation` | SubmitReview, real S3 upload, S3 notification, and confirmation status | Generated WAV | Supported |
| `evaluation-ingress` | Validation, active leases, terminal retry behavior, and idempotency conflicts | Synthetic non-dispatched S3 URIs | Supported |
| `evaluation-dispatch` | Seeded dispatch and retry through the production workflow | `--audio-file` | Supported; does not wait for completion |
| `audio-conversion` | Direct synchronous audio-processing Lambda invocation and artifact creation | `--audio-file` | Supported |
| `task-events` | WebSocket replay, live event fanout, and disconnect cleanup | Deployed `wss://` endpoint and current pipeline-event database schema | Supported |
| `task-callback` | Requires an ASR-created persisted callback attempt | Test state machine | Unsupported |
| `transcription-caller` | Isolated transcription request and persistence | External endpoint and task fixture | Unsupported |
| `moderation-caller` | Isolated moderation request and persistence | Completed transcription and task fixture | Unsupported |
| `evaluation-e2e` | Ingress, conversion, transcription, moderation, callbacks, and terminal workflow success | `--audio-file` | Supported with compatible endpoints |

The unsupported `transcription-caller`, `moderation-caller`, and `task-callback` suites fail with an explanation. Do not bypass them with placeholder task tokens or an unverified endpoint. The legacy callback test state machine cannot seed the token digest before Step Functions generates its callback token.

`task-events` and `evaluation-e2e` resolve the Terraform
`pipeline_task_events_websocket_endpoint` output into
`AUDIO_MODERATION_TASK_EVENTS_ENDPOINT`. It must be a `wss://` URL. Task-event
frames are plain UTF-8 event names for one task per socket. Delivery is
at-least-once: tests require every expected lifecycle name but tolerate duplicate
frames at the replay/live subscription boundary.

## Prerequisites

The complete deployment and `evaluation-e2e` require:

- AWS CLI credentials available through the local `admin` profile.
- Docker, Cargo, Git, Terraform, and `jq`.
- A reachable database with the current checked-in schema already applied.
- SecureString parameters for the database URL, Modal token ID, and Modal token secret.
- The Modal OIDC provider in the target AWS account.
- An HTTPS endpoint verified to accept this repository's Rust `TranscriptionRequest` contract. It defaults to the `transcription_endpoint_url` Terraform variable, so the runner can resolve it from Terraform output or the deployed transcription-caller instead of requiring `--transcription-endpoint-url`.
- An HTTPS Modal base URL whose `/moderation/` route accepts this repository's `ModerationRequest` contract. Pass it with `--modal-endpoint-url`, or let the runner resolve `modal_endpoint_url` from Terraform or the deployed moderation-caller.
- A readable, nonempty audio file for conversion or end-to-end testing.
- An applied task-events WebSocket API for the `task-events` and `evaluation-e2e`
  suites. The runner obtains its `wss://` endpoint from Terraform state unless
  `AUDIO_MODERATION_TASK_EVENTS_ENDPOINT` is explicitly set. `all task-events`
  and `all evaluation-e2e` defer this one prerequisite until after deployment.

The runner validates prerequisites but deliberately does not run database migrations or contact either external endpoint.

The callback test state machine and its IAM role are conditional Terraform resources. A normal Terraform apply leaves `enable_test_resources` at `false`; `deployment-integration.sh deploy` explicitly sets it to `true` for an integration environment.

## Common Workflows

### Validate A Full Deployment

Use this before requesting deployment approval or changing AWS resources:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

`preflight` makes read-only AWS calls. It does not build images, apply Terraform, or run tests.

### Deploy Without Tests

After explicit deployment approval:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

For unattended Terraform approval, add `--auto-approve` only when the user approved unattended deployment.

The deployment:

1. Validates the full environment.
2. Reconciles Terraform moved-resource state without deploying application resources.
3. Bootstraps all eight immutable ECR repositories.
4. Builds and pushes all eight images under one tag.
5. Performs one full Terraform apply with every image tag supplied explicitly.
6. Waits for all eight Lambda updates.

### Run An Isolated Suite

Preflight and test the suite separately when no deployment is needed:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight evaluation-ingress
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-ingress
```

Other low-dependency examples:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test review-submit
AWS_PROFILE=admin ./deployment-integration.sh test review-confirmation
AWS_PROFILE=admin ./deployment-integration.sh test task-events
```

### Test Audio Conversion

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight audio-conversion \
  --audio-file ./sample_071.mp3
AWS_PROFILE=admin ./deployment-integration.sh test audio-conversion \
  --audio-file ./sample_071.mp3
```

This uploads a source object below `reviews/integration-tests/`, directly invokes the deployed Lambda, verifies the artifact, and removes the source and artifact when the script exits.

### Test Evaluation Dispatch And Retry

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight evaluation-dispatch \
  --audio-file ./sample_071.mp3
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-dispatch \
  --audio-file ./sample_071.mp3
```

This runs the seeded undispatched-task and failed-dispatch retry cases. Both
start the production state machine and assert dispatch metadata without waiting
for terminal completion. The uploaded fixtures are retained and printed because
the asynchronous executions may still be using them.

### Deploy And Run One Suite

`all` always performs a complete eight-Lambda deployment, even when the selected suite covers only one slice:

```sh
AWS_PROFILE=admin ./deployment-integration.sh all review-confirmation \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

Do not use `all` when the user only requested testing against the existing environment.
It fails before deployment when the selected suite's fixture, secret, database,
or external-endpoint prerequisites are missing; only the WebSocket endpoint is
allowed to be absent until the new deployment produces it.

### Run End To End

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight evaluation-e2e \
  --audio-file ./sample_071.mp3 \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example

AWS_PROFILE=admin ./deployment-integration.sh test evaluation-e2e \
  --audio-file ./sample_071.mp3 \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

The suite waits up to ten minutes for the Step Functions execution to become
terminal. It then prints a structured processing report containing execution
diagnostics, persisted transcription and moderation results, produced artifact
URIs, and per-step errors. A handled pipeline failure still fails the test even
if its finalization state makes the Step Functions execution itself succeed.
Input fixtures are removed after success and retained on failure because
asynchronous work may still need them.

### Target Another Environment

Pass environment selection consistently to every command:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight evaluation-ingress \
  --region eu-west-2 \
  --project-name audio-moderation \
  --environment dev \
  --tenant-id default
```

The same settings can be supplied through `AWS_REGION`, `PROJECT_NAME`, `ENVIRONMENT`, and `TENANT_ID`. Prefer explicit flags in recorded operational instructions.

### Use An Explicit Image Tag

Normally the script creates a tag from the Git SHA and UTC timestamp. For a coordinated release, provide a new tag:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --image-tag release-2026-09-13-1 \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

ECR tags are immutable. If any repository already contains the tag, choose a new tag rather than deleting or overwriting it.

## Test Data And Cleanup

- Tests use unique identifiers to avoid idempotency collisions.
- Evaluation fixtures use `reviews/integration-tests/<run-id>/...` and never end in `/source`, so they do not trigger confirm-upload.
- `review-submit`, `review-confirmation`, and evaluation tests leave database rows for diagnosis.
- `review-confirmation` leaves its uploaded review object for the bucket lifecycle policy.
- `evaluation-dispatch` retains its input fixtures because its production workflows continue asynchronously.
- `task-callback` leaves Step Functions execution history.
- `evaluation-e2e` leaves database rows, execution history, and generated evaluation artifacts.
- `task-events` deletes its isolated pipeline task after success. On failure it
  retains that task, its durable event history, and subscription rows for diagnosis.
- Failed asynchronous evaluation fixtures are retained and printed. Do not delete them until the execution is terminal.

## Failure Handling

- If preflight fails, fix the reported prerequisite before deploying.
- If an image tag collision is reported, generate a new tag. Never mutate or remove an existing release tag to make a retry pass.
- If only some images were pushed, rerun with a new tag so all eight images form one release.
- If Terraform fails, inspect the plan/error and local state before retrying. Do not use `git reset`, delete Terraform state, or run a targeted application-resource apply as a shortcut.
- If a Lambda waiter fails, inspect the Lambda update status and CloudWatch logs before rerunning deployment.
- If an end-to-end test fails, preserve the reported S3 fixtures and Step Functions execution ARN for diagnosis.
- If the transcription endpoint rejects the Rust request contract, stop. Resolving that mismatch is a separate application-contract decision, not a deployment-script workaround.
- If the Modal moderation endpoint rejects the request contract, stop and verify its configured base URL and `/moderation/` API contract.

## Reporting Results

When reporting an agent-run workflow, include:

- The exact command and suite.
- Whether the action was preflight, deployment, testing, or both.
- AWS region, project, and environment, but no secrets.
- The release image tag for deployments.
- Which stages completed before a failure.
- Retained fixture URIs or Step Functions execution ARNs when relevant.
- Explicitly state when no deployment or test was run.
