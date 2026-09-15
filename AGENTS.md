## Migration Policy

This project has not reached production. Update existing migration files directly when changing the current schema; do not create incremental migrations solely to preserve a deployed migration history.

## Verification Policy

Do not run `git diff --check`; it is not a useful verification step for this project.

## Deployment

All agents must use the local `admin` AWS CLI profile for AWS CLI, Terraform, and deployment commands. Prefix commands with `AWS_PROFILE=admin`; do not use the default profile or another AWS account.

Use the canonical root command: `AWS_PROFILE=admin ./deployment-integration.sh`. Start with `AWS_PROFILE=admin ./deployment-integration.sh preflight`, then run `AWS_PROFILE=admin ./deployment-integration.sh deploy` for deployment only, `AWS_PROFILE=admin ./deployment-integration.sh test <suite>` for a deployed suite, or `AWS_PROFILE=admin ./deployment-integration.sh all <suite>` to deploy and run one suite. `--auto-approve` is available for unattended `deploy` and `all` commands.

Follow `docs/deployment-integration.md` for agent safety rules, prerequisites, suite selection, common workflows, cleanup behavior, and failure handling.

`deploy` bootstraps all seven ECR repositories, builds and pushes all seven Lambda images under one collision-checked immutable tag, explicitly passes every image tag to one full Terraform apply, and waits for all seven Lambdas. Never use a direct Terraform apply with `latest`; the seven image-tag variables are required. The command uses local Terraform state, so do not deploy concurrently from separate worktrees.

The selectable deployed suites, in increasing scope, are `review-submit`, `review-confirmation`, `evaluation-ingress`, `evaluation-dispatch`, `audio-conversion`, `task-callback`, `transcription-caller`, `moderation-caller`, and `evaluation-e2e`. `evaluation-ingress` covers request validation, active leases, terminal retry behavior, and idempotency conflicts with synthetic non-dispatched S3 URIs. `evaluation-dispatch` covers seeded dispatch and retry through the production state machine; it requires `--audio-file`, does not wait for terminal execution, and retains its fixtures for asynchronous work. The isolated caller suites remain unsupported because they need dedicated safe task/token fixtures. `evaluation-e2e` waits for conversion, transcription, moderation, task callback delivery, and terminal Step Functions success.

The full deployment and end-to-end suite require externally compatible HTTPS transcription and moderation endpoints, Modal OIDC, three encrypted SSM parameters, and a database with the current schema. The checked-in socialguard-models application contract is not changed to resolve the transcription endpoint mismatch. Verify an endpoint that accepts this repository's `TranscriptionRequest`, then pass `--confirm-compatible-transcription-endpoint` (or set `TRANSCRIPTION_ENDPOINT_COMPATIBLE=true`). Configure the Modal moderation base URL with `--modal-endpoint-url URL`; the caller posts to its `/moderation/` route. Preflight validates configuration without printing parameter values and deliberately does not run migrations.

Review upload and evaluation are separate business flows. There is no review-to-evaluation integration: review-confirmation checks the S3 upload notification flow, while evaluation suites submit explicit evaluation audio object URIs.
