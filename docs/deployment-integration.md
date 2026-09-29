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
- Cloudflare proxy deployment reads `CLOUDFLARE_API_TOKEN` from the environment,
  or the legacy `CLOUDFLARE_API_KEY` plus `CLOUDFLARE_EMAIL` pair; never pass
  credentials as Terraform variables or commit them to a file.
- Do not deploy or modify Modal resources without explicit approval immediately before the Modal command.
- Do not assume the checked-in `socialguard-models` gateway is compatible with the Rust `TranscriptionRequest`. A compatible external endpoint must be verified separately.

Get command details without contacting AWS:

```sh
AWS_PROFILE=admin ./deployment-integration.sh --help
```

## Business Flow Boundaries

Review upload feeds the evaluation flow:

```text
Turnstile challenge -> SubmitReview -> presigned S3 upload -> confirm-upload -> Step Functions evaluation
```

`SubmitReview` validates the Turnstile token with Cloudflare Siteverify before
creating the review or pipeline task. Its result must have a permitted frontend
hostname and the configured action. `SubmitReview` atomically creates the
idempotent pipeline task and its durable acceptance event. The upload
notification validates that existing task and
starts the evaluation. The review reaches `PROCESSING` after the Step Functions
execution is recorded. Pipeline terminal outcomes are not synchronized back to
`review_jobs` by the workflow; a later upload notification can observe an
already-terminal task and update the review then. `review-confirmation` covers
only the dispatch boundary (including the pre-created task and execution input),
not model completion or a terminal review-job status.

## Commands

| Command | Purpose | Changes AWS? |
|---|---|---:|
| `preflight [suite]` | Validate tools, identity, configuration, and external prerequisites | Read-only AWS calls |
| `deploy` | Build and deploy all eight Lambda images and the complete Terraform environment | Yes |
| `test <suite>` | Run one suite against an existing deployment | Usually; tests create real data and some invoke AWS services |
| `all [suite]` | Deploy the complete environment, then run one suite | Yes |

Without a suite, `preflight` validates the complete deployment. Without a suite, `all` runs `review-confirmation` after deployment. Before `all` changes AWS resources, it validates the selected suite's local prerequisites; the task-events WebSocket endpoint alone is deferred because the deployment creates it.

## Suites

| Suite | Coverage | Fixture | Status |
|---|---|---|---|
| `review-submit` | SubmitReview, review read, linked GetEvaluation and task-event ticket | None | Supported |
| `review-confirmation` | SubmitReview, immediate pre-upload WebSocket subscription/replay, real S3 upload and live upload event, persisted pipeline task, and Step Functions dispatch/input; second PUT observes no duplicate persisted dispatch effects | Generated WAV | Supported; does not establish delivery of the second S3 notification or wait for model completion |
| `audio-conversion` | Direct synchronous audio-processing Lambda invocation and artifact creation | `--audio-file` | Supported |
| `task-events` | WebSocket replay, live event fanout, and disconnect cleanup | Deployed `wss://` endpoint and current pipeline-event database schema | Supported |
| `task-callback` | Requires an ASR-created persisted callback attempt | Test state machine | Unsupported |
| `transcription-caller` | Isolated transcription request and persistence | External endpoint and task fixture | Unsupported |
| `moderation-caller` | Isolated moderation request and persistence | Completed transcription and task fixture | Unsupported |

The unsupported `transcription-caller`, `moderation-caller`, and `task-callback` suites fail with an explanation. Do not bypass them with placeholder task tokens or an unverified endpoint. The legacy callback test state machine cannot seed the token digest before Step Functions generates its callback token.

`task-events` and `review-confirmation` resolve the Terraform
`pipeline_task_events_websocket_endpoint` output into
`AUDIO_MODERATION_TASK_EVENTS_ENDPOINT`. It must be a `wss://` URL. Task-event
frames are plain UTF-8 event names for one task per socket. A client first
exchanges an authorized review access token for a one-time ticket, then sends
`{"action":"subscribe","ticket":<ticket>}` on the socket. Delivery is
at-least-once: tests require every expected lifecycle name but tolerate duplicate
frames at the replay/live subscription boundary.

## Prerequisites

The complete deployment requires:

- AWS CLI credentials available through the local `admin` profile.
- Docker, Cargo, Git, Terraform, and `jq`.
- A reachable database with the current checked-in schema already applied.
- Existing SecureString parameters for the database URL, Modal token ID, and
  Modal token secret. Terraform creates the fourth SecureString for Turnstile
  from its managed Cloudflare widget; it is not a preflight prerequisite on a
  first deploy. The submit-audio Lambda receives only its SSM parameter name.
- Cloudflare credentials for every full deployment, even without the optional
  API proxy. The token needs Zone Read on `cloudflare_zone_name` to resolve its
  account and Turnstile Sites Read/Write on that account to manage the widget.
  If `--enable-cloudflare-proxy` is used, also grant DNS Edit on the zone.
- `--turnstile-allowed-hostnames` (or `TURNSTILE_ALLOWED_HOSTNAMES`): nonempty
  comma-separated exact **frontend** hostnames as returned by Siteverify (for
  example `app.example.com,preview.example.com`); no scheme, port, path,
  wildcards, whitespace, or API hostname unless it also serves the widget.
  This is independent of API CORS origins and Cloudflare API proxy hostnames.
  The required Siteverify action defaults to `submit_review`; set the browser
  widget's action to match. Use `--turnstile-expected-action` to override only
  for an intentionally isolated environment.
- The Modal OIDC provider in the target AWS account.
- An HTTPS endpoint verified to accept this repository's Rust `TranscriptionRequest` contract. It defaults to the `transcription_endpoint_url` Terraform variable, so the runner can resolve it from Terraform output or the deployed transcription-caller instead of requiring `--transcription-endpoint-url`.
- An HTTPS Modal base URL whose `/moderation/` route accepts this repository's `ModerationRequest` contract. Pass it with `--modal-endpoint-url`, or let the runner resolve `modal_endpoint_url` from Terraform or the deployed moderation-caller.
- A readable, nonempty audio file for the audio-conversion suite.
- When `--enable-cloudflare-proxy` is used, the deployment also creates DNS-only
  ACM validation records and proxied CNAME records for the HTTP and WebSocket APIs.
- An applied task-events WebSocket API for the `task-events` and `review-confirmation`
  suites. The runner obtains its `wss://` endpoint from Terraform state unless
  `AUDIO_MODERATION_TASK_EVENTS_ENDPOINT` is explicitly set. `all task-events`
  and `all review-confirmation` defer this one prerequisite until after deployment.

Full preflight checks the three pre-existing SSM parameters' existence/type but
does not decrypt them, create the Turnstile widget/secret, run migrations, or
verify database schema objects. It does not contact either external endpoint or
Cloudflare Siteverify. Focused deployed review preflight checks the already
deployed, Terraform-created Turnstile SecureString.

### Turnstile setup and deployed review tests

Terraform creates a managed-mode Turnstile widget in the account owning
`cloudflare_zone_name`. Its domains are the exact frontend hostnames in
`TURNSTILE_ALLOWED_HOSTNAMES` (for example `guard.utf.lol`). It writes the
widget's secret to `/${PROJECT_NAME}/${ENVIRONMENT}/turnstile-secret-key` as
SSM `SecureString`, and exports **only** the public sitekey as
`turnstile_sitekey`. Do not pre-create the SSM parameter: if it already exists,
resolve the ownership conflict before deploying (import it only if it belongs
to this widget, or remove the old parameter deliberately). Terraform state and
backups contain the secret in plaintext; keep them local and uncommitted, restrict
access and avoid printing state or sensitive plan output. The Lambda's existing
KMS policy uses the AWS-managed `alias/aws/ssm` key; a customer-managed key
requires separate decrypt permissions.

After a successful deploy, the runner runs `ui/scripts/sync-deployment-env.sh`.
It writes the **public** sitekey and matching action to ignored
`ui/.env.local` and `ui/.env.production.local`, adds API/WebSocket endpoints
when absent, and leaves existing endpoint values and other settings intact.
Run `ui/scripts/sync-deployment-env.sh` again after an approved widget rotation
or Terraform apply; rebuild the UI because Vite reads env values at build time.
No secret belongs in any UI env file, including `.env.production.local`.

`preflight review-submit`, `preflight review-confirmation`, and both review test
commands require `TURNSTILE_TEST_TOKEN` in the caller's environment. The runner
passes it to backend deployed tests without printing it. Set it securely in the
shell environment and avoid recording it in shell history. Use a fresh real token
from the deployed widget with a production secret; real tokens expire after five
minutes and are single-use. With real tokens, run `deploy` separately, then obtain
a fresh token immediately before `preflight <review-suite>` and `test <review-suite>`.
Do not supply a real token before `all <review-suite>`: building and deploying the
eight images can outlast its five-minute validity. Each new test submission needs
a fresh real token.

Cloudflare's dummy tokens do **not** validate against the Terraform-created
widget's real secret. Deployed review suites therefore require a freshly
generated real token from the configured frontend; use mocked Siteverify
responses for local tests. See the [Siteverify reference](https://developers.cloudflare.com/turnstile/get-started/server-side-validation/).

`review-confirmation` specifically needs the current review-job, pipeline-task,
task-event, and WebSocket schemas. It loads the database URL, tenant, uploads and
artifacts buckets, and evaluation state-machine ARN from the existing deployment so
the test can inspect the persisted dispatch and call `DescribeExecution` directly.

The callback test state machine and its IAM role are conditional Terraform resources. A normal Terraform apply leaves `enable_test_resources` at `false`; `deployment-integration.sh deploy` explicitly sets it to `true` for an integration environment.

## Common Workflows

### Validate A Full Deployment

Use this before requesting deployment approval or changing AWS resources:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight \
  --turnstile-allowed-hostnames app.example.com \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

`preflight` makes read-only AWS calls. It does not build images, apply Terraform, or run tests.

### Deploy Without Tests

After explicit deployment approval:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --turnstile-allowed-hostnames app.example.com \
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
AWS_PROFILE=admin ./deployment-integration.sh preflight review-confirmation
AWS_PROFILE=admin ./deployment-integration.sh test review-confirmation
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

### Deploy And Run One Suite

`all` always performs a complete eight-Lambda deployment, even when the selected suite covers only one slice:

```sh
AWS_PROFILE=admin ./deployment-integration.sh all review-confirmation \
  --turnstile-allowed-hostnames app.example.com \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

Do not use `all` when the user only requested testing against the existing environment.
It fails before deployment when the selected suite's fixture, secret, database,
or external-endpoint prerequisites are missing; only the WebSocket endpoint is
allowed to be absent until the new deployment produces it.

### Target Another Environment

Pass environment selection consistently to every command:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight review-confirmation \
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
  --turnstile-allowed-hostnames app.example.com \
  --transcription-endpoint-url https://compatible.example/transcriptions \
  --modal-endpoint-url https://compatible.example
```

ECR tags are immutable. If any repository already contains the tag, choose a new tag rather than deleting or overwriting it.

## Test Data And Cleanup

- Tests use unique identifiers to avoid idempotency collisions.
- Audio-conversion fixtures use `reviews/integration-tests/<run-id>/...` and never end in `/source`, so they do not trigger confirm-upload.
- `review-submit` and `review-confirmation` leave database rows for diagnosis.
- `review-confirmation` leaves its uploaded object, asynchronous execution history,
  database rows, and any generated evaluation artifact for diagnosis/lifecycle cleanup.
- `task-callback` leaves Step Functions execution history.
- `task-events` deletes its isolated pipeline task after success. On failure it
  retains that task, its durable event history, and subscription rows for diagnosis.
- Failed asynchronous evaluation fixtures are retained and printed. Do not delete them until the execution is terminal.

## Failure Handling

- If preflight fails, fix the reported prerequisite before deploying.
- If an image tag collision is reported, generate a new tag. Never mutate or remove an existing release tag to make a retry pass.
- If only some images were pushed, rerun with a new tag so all eight images form one release.
- If Terraform fails, inspect the plan/error and local state before retrying. Do not use `git reset`, delete Terraform state, or run a targeted application-resource apply as a shortcut.
- If a Lambda waiter fails, inspect the Lambda update status and CloudWatch logs before rerunning deployment.
- If review confirmation fails, preserve its S3 fixture and Step Functions execution ARN for diagnosis.
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
