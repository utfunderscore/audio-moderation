# SocialGuard

Monorepo for the SocialGuard audio moderation platform.

## Layout

- `backend/` — Rust AWS Lambda implementation (public API, task pipeline,
  transcription/moderation callers, task-event WebSocket). The Cargo workspace
  root is `backend/Cargo.toml`.
- `models/` — Python model services deployed to Modal (GPU transcription and
  moderation inference). See `models/README.md`.
- `ui/` — front-end application (not started yet).
- `proto/` — protobuf definitions for the public API, shared across components.
- `migrations/` — database migrations.
- `terraform/` — infrastructure as code.
- `docs/` — deployment and integration documentation.
- `deployment-integration.sh` — canonical AWS deployment and deployed-test entry
  point.

## Backend

Build and test from `backend/`:

```sh
cd backend
cargo test
```

Deploy and run a deployed integration suite from the repository root:

```sh
AWS_PROFILE=admin ./deployment-integration.sh preflight
AWS_PROFILE=admin ./deployment-integration.sh deploy
```

See `docs/deployment-integration.md` for the full workflow and safety rules.

## Models

The Python services are deployed to Modal and depend on GPU model weights.
See `models/README.md` for development, deployment, and model setup.

## Integration boundary

The backend and models are separate deployables connected by a network
contract:

- `backend/` `transcription-caller` and `moderation-caller` POST to
  `${MODAL_ENDPOINT_URL}/transcription/` and `/moderation/` with Modal proxy
  credentials and an S3 audio URI, and receive a queued task ID.
- `models/` returns terminal results to the backend's task-callback Lambda,
  which resumes the Step Functions workflow.

There is no code-level linkage between the two; keep the JSON contracts in sync
when either side changes.
