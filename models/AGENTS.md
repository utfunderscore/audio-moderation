## Local and remote verification

Run from `models/` (Python >=3.12, locked with `uv.lock`):
- `uv sync`, `uv run pytest`, `uv run basedpyright`.
  Focus with `uv run pytest tests/test_callbacks.py -k <test_filter>`.
  Basedpyright checks `src/` in strict mode.
- Local tests do not substitute for real GPU/image checks. `README.md` documents
  image-import verifiers, weight preloading, and direct inference scripts.
  `scripts/transcribe.py` and `scripts/moderate.py` bypass HTTP, idempotency,
  S3 presigning, and callbacks; they do not verify the end-to-end integration.
- Apply root deployment approval rules before remote Modal commands.

## Execution boundaries

- `src/socialguard_models/cpu_worker.py` is the shared HTTP deployment entrypoint
  for transcription and moderation. Flow: family API → `api/submission.py` →
  idempotency claim → `orchestration.py` CPU worker → family GPU job → callback.
- `ModelJob` in `model_job.py` is a serializable adapter: module-level classes
  carry task data, not live models, credentials, or clients. `submit()` returns
  a submitted call handle; shared orchestration owns waiting, cancellation,
  audio access, deadlines, and callback delivery.
- Keep GPU dependencies inside the individual model's Modal image, not local
  `pyproject.toml` or the shared CPU image.
- Adding transcription models requires the identifier in
  `transcription/contracts.py` and dispatch in `transcription/job.py`.
  Moderation uses `moderation/models/registry.py`; register models in source so
  API and CPU containers load the same registry, not during request handling.
- Family-specific callback mappings are in `transcription/callbacks.py` and
  `moderation/callbacks.py`; shared signing/retries are in `callbacks.py`.
  Check the Rust consumer when changing payloads: README passages describing
  moderation callbacks as “proposed” are not authoritative consumer support checks.

## Runtime configuration

- Both families use the existing Modal app `socialguard-transcription` and named
  Secret `socialguard-transcription-runtime`, defined in `modal_app.py`.
  Deployment-shell variables are not automatically injected into containers.
- The runtime secret supplies `AWS_REGION`, `AWS_ROLE_ARN`, and
  `TASK_CALLBACK_FUNCTION_NAME`; the role uses Modal OIDC for S3 reads and
  direct callback invocation. Follow `README.md` for secret/environment setup
  and persistent model-cache volumes.
- Moderation returns five model-defined category scores, not a combined verdict
  or calibrated probabilities. Preserve their semantics from
  `moderation/contracts.py` and the model-specific score mapping.
