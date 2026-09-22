# What the Deployment Script Builds and Updates

`deployment-integration.sh deploy` packages the application as eight Lambda
container images, publishes them to ECR, and runs Terraform to update the complete
AWS environment. Operational commands and safety rules are documented separately in
[`deployment-integration.md`](deployment-integration.md).

## Deployment at a glance

```text
Rust crates
   │
   ├── build eight linux/amd64 container images
   │
   ├── push all images to ECR under one immutable tag
   │
   └── pass that tag to one full Terraform apply
             │
             ├── update eight Lambda functions
             ├── update API Gateway routes and integrations
             ├── update Step Functions and event routing
             ├── update S3 buckets and notifications
             └── update IAM, logs, permissions, and supporting resources
```

All eight images use the same release tag. This keeps the deployed Lambdas on one
coherent version of the codebase.

## Images that are built

| Image | Purpose |
|---|---|
| `submit-audio` | Accepts review submissions and creates presigned S3 upload details. |
| `confirm-upload` | Handles matching S3 upload notifications and starts review evaluations. |
| `audio-processing` | Downloads input audio, stitches it, writes the artifact, and finalizes workflow outcomes. |
| `start-evaluation` | Accepts evaluation requests, creates pipeline tasks, and starts Step Functions executions. |
| `task-callback` | Receives external transcription and moderation callbacks and resumes Step Functions tasks. |
| `transcription-caller` | Starts an external transcription request. |
| `moderation-caller` | Starts an external moderation request. |
| `task-events` | Handles WebSocket connections and subscriptions and replays persisted task events. |

All eight binaries are built by a single consolidated Dockerfile,
`backend/Dockerfile.lambda`. Its shared `builder` stage compiles every package
in one `cargo build`, then each runtime stage copies its binary with
`docker build --target`. The script builds every image for `linux/amd64`, even
if only one Lambda changed, and BuildKit caches the cargo registry and target
directory across deploys.

## ECR updates

Before building images, Terraform ensures that all eight ECR repositories exist.
Each repository has:

- an immutable-tag policy;
- permission for Lambda to pull images; and
- a lifecycle policy retaining the most recent images.

The script then pushes one image to each repository using the same tag. It refuses
to reuse a tag that already exists in any repository, so a release cannot be
partially overwritten.

## Lambda updates

The full Terraform apply points these eight Lambda functions at the newly pushed
images:

```text
<project>-<environment>-submit-audio
<project>-<environment>-confirm-upload
<project>-<environment>-audio-processing
<project>-<environment>-start-evaluation
<project>-<environment>-task-callback
<project>-<environment>-transcription-caller
<project>-<environment>-moderation-caller
<project>-<environment>-task-events
```

Terraform also updates each function's configuration, including environment
variables, timeout and memory settings, IAM role, API or event permissions, and
CloudWatch log group. After Terraform finishes, the script waits until AWS reports
that all eight function updates are complete.

When `--enable-cloudflare-proxy` is supplied, Terraform also requests a free ACM
certificate, creates API Gateway custom domains, disables the default
`execute-api` endpoints, and publishes proxied Cloudflare CNAME records. The
Cloudflare provider reads `CLOUDFLARE_API_TOKEN` from the environment, or the
legacy `CLOUDFLARE_API_KEY` plus `CLOUDFLARE_EMAIL` pair; credentials are never
passed as Terraform variables.

## API Gateway updates

Terraform manages two APIs:

### Public HTTP API

The HTTP API routes requests to:

- `submit-audio` for review submission;
- `start-evaluation` for evaluation ingress; and
- `task-callback` for external task callbacks.

The deployment updates the Lambda integrations, routes, default stage, throttling,
and Lambda invocation permissions.

### Task-events WebSocket API

The WebSocket API routes:

- `$connect`;
- `subscribe`; and
- `$disconnect`

to the `task-events` Lambda. Terraform updates the API, stage, Lambda integration,
invocation permission, and the management-API permissions used to send event frames
back to connected clients. The `subscribe` route accepts a one-time ticket minted
by the authorized `CreateTaskEventsTicket` HTTP RPC; it does not accept a task ID.

## Workflow and event updates

Terraform manages the production Step Functions workflow that runs:

```text
audio processing → transcription → moderation → finalization
```

The state machine definition references the deployed Lambda functions. Terraform
also updates:

- its execution role and Lambda invocation permissions;
- its CloudWatch logging configuration;
- the EventBridge rule that reconciles terminal Step Functions executions; and
- the EventBridge permission to invoke `audio-processing` for finalization.

The integration deployment enables the test-only callback state machine as well.
That resource is isolated from the production workflow.

## S3 updates

Terraform manages two private, encrypted buckets:

- the uploads bucket for source audio; and
- the artifacts bucket for stitched evaluation audio.

It applies public-access blocking, encryption, retention rules, and incomplete-upload
cleanup. It also manages the uploads-bucket notification that invokes
`confirm-upload` for matching review source objects, together with the corresponding
Lambda permission.

## IAM and supporting AWS resources

The apply updates the supporting resources required by the application:

- one execution role and associated policies for each Lambda;
- permissions for SSM parameters, KMS decryption, S3 objects, Step Functions,
  callback invocation, and WebSocket connection management;
- the Step Functions execution roles;
- the Modal federated role that can read stitched artifacts and invoke callbacks;
- CloudWatch log groups and retention settings; and
- the tagged AWS Resource Group for the environment.

The Modal OIDC provider itself is not created by this deployment; Terraform expects
the configured provider to already exist.

## What the deployment does not update

The script does not:

- apply PostgreSQL migrations or alter the database schema;
- create or update the external transcription service;
- deploy the Modal moderation service;
- create the encrypted SSM parameter values; or
- replace the Modal OIDC provider.

Those are prerequisites consumed by the deployed Lambdas.

## Useful commands

Run every command from the repository root with the `admin` AWS profile.

### Deploy the complete AWS environment

Build and push all eight images, apply the full Terraform configuration, and wait
for every Lambda update:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

To expose the APIs through Cloudflare:

```sh
CLOUDFLARE_API_TOKEN=<redacted> AWS_PROFILE=admin \
  ./deployment-integration.sh deploy \
  --enable-cloudflare-proxy \
  --cloudflare-zone-name utf.lol \
  --public-api-domain-name api-guard.utf.lol \
  --task-events-domain-name events-guard.utf.lol \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

Terraform normally asks for approval. For an already-approved unattended deployment:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example \
  --auto-approve
```

### Deploy with a known image tag

Use a release tag when the exact ECR image version needs to be recorded or shared:

```sh
AWS_PROFILE=admin ./deployment-integration.sh deploy \
  --image-tag release-2026-09-16-1 \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

Tags are immutable. A previously pushed tag cannot be reused.

### Run a suite against the existing deployment

This does not rebuild images or apply Terraform:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test task-events
```

Other useful isolated suites include:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test review-submit

AWS_PROFILE=admin ./deployment-integration.sh test review-confirmation

AWS_PROFILE=admin ./deployment-integration.sh test evaluation-ingress
```

### Test audio conversion

Upload a real audio fixture, invoke the deployed audio-processing Lambda, verify the
artifact, and clean up the synchronous fixture:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test audio-conversion \
  --audio-file ./sample_071.mp3
```

### Run the complete evaluation flow

Run the existing deployment through audio processing, transcription, moderation,
callbacks, terminal workflow handling, and WebSocket lifecycle delivery:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test evaluation-e2e \
  --audio-file ./sample_071.mp3 \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

### Deploy and immediately run one suite

`all` always deploys the complete eight-Lambda environment before running the
selected suite:

```sh
AWS_PROFILE=admin ./deployment-integration.sh all task-events \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

To deploy and run the full evaluation suite:

```sh
AWS_PROFILE=admin ./deployment-integration.sh all evaluation-e2e \
  --audio-file ./sample_071.mp3 \
  --transcription-endpoint-url https://transcription.example/transcriptions \
  --modal-endpoint-url https://moderation.example
```

### Target another environment

The region, project name, environment, and tenant determine the Terraform and AWS
resource names selected by the runner:

```sh
AWS_PROFILE=admin ./deployment-integration.sh test task-events \
  --region eu-west-2 \
  --project-name audio-moderation \
  --environment staging \
  --tenant-id default
```

## Integration tests after deployment

Running `all <suite>` performs the full deployment above and then executes one
selected suite. Tests may upload temporary S3 objects, create database rows, open
WebSocket connections, invoke Lambdas, or start Step Functions executions, but those
are test fixtures and executions rather than additional application infrastructure.

`review-confirmation` validates the upload-to-dispatch boundary only: it verifies
the persisted idempotent task and Step Functions input after a real upload. It then
performs a second real PUT and observes no duplicate persisted dispatch effects for
the notification-delivery window. Because the test cannot observe S3 delivery to
the Lambda, it does not prove that the second notification was delivered. It does
not wait for transcription, moderation, model completion, or terminal review-job
status.
