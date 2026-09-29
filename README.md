# SocialGuard

Monorepo for the SocialGuard audio moderation platform.

## Layout

- `backend/` — Rust AWS Lambda implementation (public API, task pipeline,
  transcription/moderation callers, task-event WebSocket). The Cargo workspace
  root is `backend/Cargo.toml`.
- `models/` — Python model services deployed to Modal (GPU transcription and
  moderation inference). See `models/README.md`.
- `ui/` — React/Vite browser application with Turnstile-protected audio submission.
  See `ui/README.md` for API and public sitekey configuration.
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
export TURNSTILE_ALLOWED_HOSTNAMES=guard.utf.lol,localhost,127.0.0.1
AWS_PROFILE=admin ./deployment-integration.sh preflight
AWS_PROFILE=admin ./deployment-integration.sh deploy
```

Set `TURNSTILE_ALLOWED_HOSTNAMES` to the exact frontend widget hostnames for
preflight/deploy and provide Cloudflare credentials. Add any other development
hostname used in the browser; omit development hostnames in production. Use
hostnames without a scheme or port. Terraform creates the
widget and its SSM SecureString, then the runner writes the public sitekey to
ignored UI env files. See
`docs/deployment-integration.md` for Turnstile setup, deployed test tokens, the
full workflow, and safety rules.

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
