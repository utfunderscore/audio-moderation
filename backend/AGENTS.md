## Build and verification

- Run Cargo commands from `backend/`. `.cargo/config.toml` defaults
  `SQLX_OFFLINE=true`; merely passing `--manifest-path backend/Cargo.toml` from
  the repository root does not load this directory's Cargo configuration.
- `cargo test` checks the workspace; focus with `cargo test -p <crate> <filter>`.
  Example database target: `cargo test -p database --test pipeline_task_store_test`.
- Database integration tests use testcontainers with PostgreSQL 17 and apply
  root `migrations/` to isolated containers. Docker must be running; starting
  the root Compose database does not replace this prerequisite.
- Audio stitcher tests require `ffmpeg` on PATH. Lambda's image supplies its own
  FFmpeg binary, so a successful container build does not supply the local tool.
- SQLx query macros compile against checked-in `.sqlx/` metadata. Keep that
  cache in sync when changing queries/schema; offline compilation alone does
  not verify compatibility with a running database.
- `tests/deployed.rs` targets are ignored by default. Run them through root
  `deployment-integration.sh`, which resolves endpoints, secrets, and fixtures;
  do not enable all ignored tests as a routine local verification step.

## Contracts and wiring

- `submit-audio-lambda` and `start-evaluation-lambda` generate Connect RPC types
  from root `proto/` in their build scripts. Edit the proto sources, not generated
  output. Schema sources also live outside this workspace, in root `migrations/`.
- Workflow wiring lives in root Terraform alongside Lambda infrastructure;
  inspect it when changing step inputs, outputs, or callback behavior.
- `database` owns persistence; `task-event-emitter` owns shared lifecycle event
  emission. `task-events-lambda` serves one task per WebSocket with durable replay
  and live fanout. Frames are UTF-8 event names and delivery is at-least-once.
- Transcription/moderation callers submit asynchronous jobs to the Python service;
  acceptance is not completion. `task-callback-lambda` resumes the workflow using
  persisted callback-attempt state. See root `docs/task-callback.md` before changing
  token handling or callback contracts.
- Lambda Dockerfiles need repository-root build context: they copy `backend/`,
  `proto/`, and `migrations/`. Use the root deployment runner for image publication.

## Deployed test selection

- Follow root `docs/deployment-integration.md` and the root `AGENTS.md` deployment
  rules. Crate READMEs add details for start-evaluation and task-events tests.
- `evaluation-ingress` uses synthetic non-dispatched URIs; dispatch/E2E tests need
  real audio. `review-confirmation` verifies that an uploaded review creates one
  idempotent pipeline task and dispatches the expected Step Functions execution.
  It then repeats the PUT and observes no duplicate persisted dispatch effects for
  its delivery window; it cannot prove S3 delivered that second notification. It
  does not wait for model completion or a terminal review-job status.
