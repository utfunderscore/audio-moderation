## Boundaries that matter

- Component commands and implementation gotchas live in `backend/AGENTS.md`,
  `models/AGENTS.md`, and `ui/AGENTS.md`. Cargo's workspace root is `backend/`.
- Review upload notifications create an idempotent pipeline task and start the
  Step Functions conversion → transcription → moderation flow. Public callers
  can also start that flow with explicit audio object URIs through
  `start-evaluation-lambda`.
- `models/` is a separate Modal deployable. Rust callers and Python services share
  JSON request/callback contracts, not code. Keep both sides in sync; terminal
  callbacks go through `task-callback-lambda` to resume Step Functions.
- Public RPC sources live in root `proto/`; database schema lives in `migrations/`.
- `ui/` is a React/Vite browser-only demo, despite the root README's placeholder
  description. `usePipelineRun` wires `MockTaskEventsClient`; transcripts and scores
  are fixtures. Real task-event WebSocket frames contain event names only, with
  at-least-once replay/live delivery. See `ui/README.md` for the transport boundary.

## Shared policies and local database

- Do not run `git diff --check` (repository policy).
- Pre-production policy: edit existing files in `migrations/` for current-schema
  changes; do not add incremental migrations solely to preserve deployment history.
- Root `compose.yaml` provides local PostgreSQL and a Flyway migration service;
  database tests instead use their own isolated containers.

## Deployment and deployed tests

- Prefix **all** AWS CLI, Terraform, and deployment commands with `AWS_PROFILE=admin`.
- Read `docs/deployment-integration.md` before remote operations; it defines
  approval requirements, prerequisites, suite selection, and fixture cleanup.
  AWS deployment requires explicit user approval; Modal modifications require
  explicit approval immediately before the command.
- From the root, start with `AWS_PROFILE=admin ./deployment-integration.sh preflight`
  (or `preflight <suite>` for focused testing). Then use the same prefix with
  `deploy`, `test <suite>`, or `all <suite>`. `all` deploys all eight Lambdas even
  for a narrow suite; use `test` for an existing deployment.
- Use the runner rather than direct Terraform apply or ad hoc image publication.
  It coordinates all eight images under one immutable tag; never use `latest`.
  Terraform state is local: do not deploy concurrently from separate worktrees.
- Preflight checks that required SSM parameters exist as `SecureString` values, but
  neither decrypts them nor validates the evaluation secret's 32-byte minimum. It
  also neither applies migrations nor verifies database schema objects, and does
  not contact model endpoints. Full deploy and E2E need the current database
  schema, Modal OIDC, four SecureString SSM parameters, and compatible HTTPS
  endpoints. `--modal-endpoint-url` is a base URL; moderation appends
  `/moderation/`. Never print decrypted secrets.
- `task-callback`, `transcription-caller`, `moderation-caller`, `evaluation-ingress`,
  and `evaluation-dispatch` isolated suites are unsupported; do not bypass them with
  placeholder task tokens or synthetic ingress fixtures. `evaluation-e2e` submits a
  real review upload, waits for terminal workflow persistence, and does not depend on
  task-event WebSocket delivery; see the runbook for source cleanup on success/failure.
