## Boundaries that matter

- Component commands and implementation gotchas live in `backend/AGENTS.md`,
  `models/AGENTS.md`, and `ui/AGENTS.md`. Cargo's workspace root is `backend/`.
- Review submission creates an idempotent pipeline task; upload notifications
  start the Step Functions conversion → transcription → moderation flow.
- `models/` is a separate Modal deployable. Rust callers and Python services share
  JSON request/callback contracts, not code. Keep both sides in sync; terminal
  callbacks go through `task-callback-lambda` to resume Step Functions.
- Public RPC sources live in root `proto/`; database schema lives in `migrations/`.
- `ui/` is a React/Vite browser demo. Its `ApiBackend` owns RPC, upload, and
  task-event transport behind the `Backend` interface. Task-event WebSocket
  frames contain event names only, with at-least-once replay/live delivery;
  transcripts and scores come from the evaluation read API. See `ui/README.md`.

## Shared policies and local database

- Do not run `git diff --check` (repository policy).
- Pre-production policy: edit existing files in `migrations/` for current-schema
  changes; do not add incremental migrations solely to preserve deployment history.
- Root `compose.yaml` provides local PostgreSQL and a Flyway migration service;
  database tests instead use their own isolated containers.

## Deployment and deployed tests

- Prefix **all** AWS CLI, Terraform, and deployment commands with `AWS_PROFILE=admin`.
- Read `scripts/deployment/README.md` before deployment or
  `scripts/deployed-tests/README.md` before remote tests; they define approval
  requirements, prerequisites, suite selection, and fixture cleanup.
  AWS deployment requires explicit user approval; Modal modifications require
  explicit approval immediately before the command.
- From the root, use `AWS_PROFILE=admin ./scripts/deploy.sh preflight`, then
  `deploy`, or `AWS_PROFILE=admin ./scripts/test-deployed.sh preflight <suite>`,
  then `test <suite>`. Both workflows automatically preflight their execution.
  `deployment-integration.sh` is a legacy dispatcher; its `all` command still
  deploys all eight Lambdas even for a narrow suite.
- Use the runner rather than direct Terraform apply or ad hoc image publication.
  It coordinates all eight images under one immutable tag; never use `latest`.
  Terraform state is local: do not deploy concurrently from separate worktrees.
- Workflow-specific prerequisites and implementation instructions live in
  `scripts/deployment/AGENTS.md` and `scripts/deployed-tests/AGENTS.md`.
  Never print decrypted secrets or test tokens. Terraform state contains the
  widget secret; never commit it.
- `task-callback`, `transcription-caller`, and `moderation-caller` isolated suites
  are unsupported; do not bypass them with placeholder task tokens.
  `review-confirmation` tests upload-triggered dispatch but does not wait for
  workflow completion; see the runbook for fixture cleanup.
