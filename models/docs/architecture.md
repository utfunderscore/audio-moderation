# Architecture

## HTTP API

- [`api/app.py`](../src/socialguard_models/api/app.py) — FastAPI application factory exposing:
  - `POST /v1/transcriptions`
  - `GET /v1/models`
  - `GET /healthz`
  - authentication, typed errors, idempotency conflict handling, and callback OpenAPI metadata.

- [`api/schemas.py`](../src/socialguard_models/api/schemas.py) — Strict Pydantic request, response, error, and callback models. Defines the supported Granite model and terminal callback event types.

- [`api/auth.py`](../src/socialguard_models/api/auth.py) — Bearer-token authentication using `SOCIALGUARD_GATEWAY_API_TOKEN` and constant-time comparison.

- [`api/generate_openapi.py`](../src/socialguard_models/api/generate_openapi.py) — Deterministically generates the committed [`gateway.openapi.json`](../openapi/gateway.openapi.json).

## Job submission and reliability

- [`api/submission.py`](../src/socialguard_models/api/submission.py) — Caller-scoped idempotency and dispatch coordination. It hashes request payloads, returns the original transcription ID for identical retries, rejects conflicting key reuse, prevents concurrent duplicate dispatch, and returns `503` unless dispatch is confirmed.

- [`api/callbacks.py`](../src/socialguard_models/api/callbacks.py) — Serializes and signs terminal callbacks using:

  ```text
  HMAC-SHA256(key, ASCII(timestamp) + "." + exact_body)
  ```

  Implements bounded retries and accepts any HTTP 2xx response.

- [`api/networking.py`](../src/socialguard_models/api/networking.py) — URL and SSRF policy. Requires HTTPS port 443 and public IP resolution, and rejects credentials, fragments, private networks, and unsafe destinations.

## Shared model contract

- [`core/models.py`](../src/socialguard_models/core/models.py) — Immutable model-neutral values for audio inputs, transcription options, transcripts, and model identity.

- [`core/transcriber.py`](../src/socialguard_models/core/transcriber.py) — Structural `Transcriber` protocol for future backend adapters.

- [`core/capabilities.py`](../src/socialguard_models/core/capabilities.py) — Supported transcription options and response-format declarations.

## Modal deployment

- [`deployments/granite.py`](../src/socialguard_models/deployments/granite.py) — Main Modal App containing:
  - lightweight CPU FastAPI gateway;
  - Modal Dict job and idempotency ledger;
  - CPU download and callback orchestration worker;
  - lifecycle-loaded L40S Granite GPU class;
  - 25 MiB download and 60-second audio limits;
  - mono 16 kHz normalization and deterministic inference.

- [`deployments/granite_resources.py`](../src/socialguard_models/deployments/granite_resources.py) — Shared pinned configuration for the Granite model revision, L40S resources, Hugging Face cache Volume, and locked runtime image.

- [`deployments/granite_prefetch.py`](../src/socialguard_models/deployments/granite_prefetch.py) — Downloads the pinned model snapshot into the Modal Volume. It is the only cache writer.

- [`deployments/granite_smoke.py`](../src/socialguard_models/deployments/granite_smoke.py) — Standalone paid GPU smoke test using IBM's bundled audio fixture.

## Request flow

```text
Client POST
  → FastAPI authentication and validation
  → Modal Dict idempotency record
  → spawned CPU orchestration worker
  → bounded audio download
  → L40S Granite inference
  → immutable terminal event
  → signed, retryable HTTPS callback
```

## Configuration and tests

- [`pyproject.toml`](../pyproject.toml) — Dependencies, Granite dependency group, Ruff, basedpyright, and pytest configuration.
- [`tests/gateway/test_gateway.py`](../tests/gateway/test_gateway.py) — HTTP and OpenAPI contract tests.
- [`tests/unit/test_gateway_runtime.py`](../tests/unit/test_gateway_runtime.py) — Idempotency, concurrency, URL policy, and callback signing tests.
- [`tests/unit/test_granite_modal.py`](../tests/unit/test_granite_modal.py) — Modal registration and resource configuration checks.
