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
- `scripts/deploy.sh` — canonical coordinated AWS deployment entry point.
- `scripts/test-deployed.sh` — suite-specific tests of an existing deployment.
- `deployment-integration.sh` — legacy compatibility dispatcher.

## Backend

Build and test from `backend/`:

```sh
cd backend
cargo test
```

Deploy from the repository root:

```sh
export TURNSTILE_ALLOWED_HOSTNAMES=guard.utf.lol,localhost,127.0.0.1
AWS_PROFILE=admin ./scripts/deploy.sh preflight
AWS_PROFILE=admin ./scripts/deploy.sh deploy
```

Set `TURNSTILE_ALLOWED_HOSTNAMES` to the exact frontend widget hostnames for
preflight/deploy and provide Cloudflare credentials. Add any other development
hostname used in the browser; omit development hostnames in production. Use
hostnames without a scheme or port. Terraform creates the
widget and its SSM SecureString, then the runner writes the public sitekey to
ignored UI env files. See
`scripts/deployment/README.md` for deployment prerequisites and safety rules.

Test an existing deployment without changing infrastructure:

```sh
AWS_PROFILE=admin ./scripts/test-deployed.sh preflight task-events
AWS_PROFILE=admin ./scripts/test-deployed.sh test task-events
```

See `scripts/deployed-tests/README.md` for suite selection, Turnstile test tokens,
and fixture cleanup. Runner regression tests are local and use mocked tools:
`python3 -m unittest discover -s scripts/tests -v`.

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
