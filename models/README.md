# socialguard-models

Typed ASR model deployments for Modal.

## Requirements

- Python 3.12
- [`uv`](https://docs.astral.sh/uv/)

## Set up

```bash
uv sync
```

`uv sync` creates `.venv`, installs this package, and installs the default `dev`
dependency group.

## Validate

```bash
uv lock --check
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
```

## Asynchronous gateway contract

The Modal gateway accepts `POST /v1/transcriptions` with a JSON body containing:

- `model` — a supported model type;
- `audio_url` — an HTTPS presigned download URL for the audio file;
- `callback_url` — an HTTPS webhook endpoint for completion or failure events;
- `report_id` — the caller's opaque report identifier, echoed in the `202` response
  and callback events; and
- `idempotency_key` — a caller-generated retry key, distinct from `report_id`.

The generated public definition is committed at
[`openapi/gateway.openapi.json`](openapi/gateway.openapi.json). Set
`SOCIALGUARD_GATEWAY_API_TOKEN` in the gateway environment and send it as
`Authorization: Bearer <token>` for `/v1/*` routes; `/healthz` remains unauthenticated
for platform liveness probes. Idempotency is scoped to that authenticated caller:
identical retries return the original accepted job, while reuse with a different payload
returns a typed `409 idempotency_conflict` response.

Callbacks are at-least-once and receivers must deduplicate by gateway transcription ID.
The delivery worker signs `ASCII(timestamp) + b'.' + exact JSON body bytes` using
HMAC-SHA256 and sends `X-SocialGuard-Timestamp`, `X-SocialGuard-Webhook-Key-Id`, and
`X-SocialGuard-Signature: sha256=<lowercase-hex>` headers. Any 2xx response
acknowledges delivery. Each worker attempt makes three callback attempts with one- and
two-second backoff (a three-second window); two Modal retries redeliver the exact
persisted bytes after callback exhaustion.

The deployed gateway uses a bounded Modal Dict ledger. Its idempotency and recovery
records expire after seven days of inactivity, and a ledger write cannot be transactional
with spawning a Modal worker. Concurrent identical requests use a bounded handoff:
they return `202` only after one request has spawned work or the ledger confirms it was
dispatched. A temporary `503` is safe to retry with the same idempotency key. Use a
transactional external database before requiring longer retention or stronger outbox
guarantees. Audio downloads are limited to 25 MiB compressed bytes and 60 seconds decoded duration.
The worker allows 90 seconds for download, 180 seconds for GPU inference, and 420 seconds
total; the GPU gets a 600-second cold-start budget. URL dereferencing rejects non-HTTPS,
non-443, credential-bearing, fragment-bearing, and private-network destinations. Python HTTP clients cannot pin the
validated DNS address, so deployment egress allowlists remain the defense against DNS
rebinding.

### Deploy prerequisites

Create two Modal Secrets before deploying: `socialguard-gateway-api` containing
`SOCIALGUARD_GATEWAY_API_TOKEN`, and `socialguard-callback-signing` containing
`SOCIALGUARD_CALLBACK_HMAC_KEY` plus `SOCIALGUARD_CALLBACK_KEY_ID`. Prefetch the
pinned model cache before deployment. The deployment entrypoint is
`socialguard_models.deployments.granite`; it has a CPU ASGI gateway, CPU orchestrator,
and lifecycle-loaded L40S worker with one active inference per container.

## Dependency policy

Shared HTTP and Modal dependencies belong in `[project.dependencies]`.
Development-only tools belong in `[dependency-groups].dev`.
Each ASR backend will receive its own dependency group so heavy model frameworks are
not installed in the default development environment.

## Granite GPU spike

The first backend spike uses the immutable Granite revision
`bd87ab862416353633ea431fe49b1614003623c5` and a named Modal Volume. Its heavy
runtime is isolated in the `backend-granite` group.

Prefetch the model once before the GPU smoke test. This downloads approximately 6.35 GB
into your Modal Volume and can incur storage/network cost:

```bash
uv run modal run \
  -m socialguard_models.deployments.granite_prefetch::prefetch_model
```

Run one paid L40S smoke transcription after prefetch succeeds:

```bash
uv run modal run \
  -m socialguard_models.deployments.granite_smoke::smoke_transcription
```

These commands use only the lightweight local Modal CLI. The Modal image installs the
`backend-granite` group remotely.

The smoke test uses the model's pinned `multilingual_sample.wav`, BF16, one active
input, and deterministic generation (`max_new_tokens=200`, no sampling, one beam).
It does not expose an HTTP endpoint or implement the Granite backend adapter.
