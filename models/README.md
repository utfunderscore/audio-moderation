# SocialGuard Models

Python models and integrations for SocialGuard.

The transcription service currently supports
[`ibm-granite/granite-speech-4.1-2b`](https://huggingface.co/ibm-granite/granite-speech-4.1-2b).
It returns plain transcript text with punctuation and capitalization. It does not
request or return speaker attribution, timestamps, or transcript segments.

## Development

Install dependencies:

```sh
uv sync
```

Run tests and type checking:

```sh
uv run pytest
uv run basedpyright
```

## Transcription API

`src/socialguard_models/api/transcription_api.py` defines the `POST /` route.
`src/socialguard_models/cpu_worker.py` serves it as a FastAPI application and
defines the shared CPU deployment. The route runs orchestration on CPU and
submits inference to a separate GPU worker:

```text
HTTP endpoint + transcription control plane (CPU)
  -> selected transcription worker (GPU)
  <- completed or failed outcome
<- HTTP response
```

The CPU deployment uses one container (`max_containers=1`) with up to 32 concurrent
requests (`modal.concurrent(max_inputs=32)`). Each request runs on its own thread,
so waiting for a GPU result does not block other requests. Requests above that
limit queue; GPU workers scale separately. The CPU container scales to zero when
idle. Adjust `MAX_CONCURRENT_REQUESTS` in `cpu_worker.py` to tune concurrency.

The endpoint calls `run_transcription()` in `src/socialguard_models/transcription.py`
directly. That function creates a short-lived S3 URL using Modal OIDC credentials,
selects the GPU worker with an explicit `match` on the model, submits it with
`spawn()`, and waits for its result within a deadline that includes preparation
and submission time. Failures produce a failed outcome and trigger best-effort
cancellation if a worker was submitted.

The function logs success or failure without transcript text, task tokens, or
exception messages. The endpoint converts its outcome into the HTTP response.

Granite runs on an L4 GPU and normalizes any FFmpeg-supported audio input to
mono 16 kHz. Inputs are limited to 512 MiB and five minutes of decoded audio.
The duration limit keeps audio embeddings and generated text within the model's
context window while bounding temporary disk, memory, and inference time.

The AWS role named by `AWS_ROLE_ARN` must trust Modal's OIDC provider and permit
`s3:GetObject` for input objects. The endpoint also requires Modal proxy
authentication.

Preload the pinned Granite model revision into the automatically created
`socialguard-transcription-models` Modal volume:

```sh
uv run modal run src/socialguard_models/granite.py::download_granite_model
```

This step avoids downloading approximately 9.5 GB of model files during the
first inference cold start. The inference container can still populate an empty
cache if preloading is skipped.

Run the endpoint during development:

```sh
uv run modal serve src/socialguard_models/cpu_worker.py
```

Deploy it with a persistent URL:

```sh
uv run modal deploy src/socialguard_models/cpu_worker.py
```

Send a transcription request to the URL printed by Modal:

```sh
curl -X POST "$MODAL_ENDPOINT_URL" \
  -H "Content-Type: application/json" \
  -H "Modal-Key: $MODAL_PROXY_KEY" \
  -H "Modal-Secret: $MODAL_PROXY_SECRET" \
  -d '{
    "model": "granite",
    "audio_uri": "s3://example/audio.wav",
    "idempotency_key": "request-123",
    "pipeline_task_id": "task-123",
    "task_token": "token-123"
  }'
```

The request remains open while transcription runs and returns a completed
response:

```json
{
  "status": "completed",
  "model": "granite",
  "text": "The transcribed speech appears here.",
  "idempotency_key": "request-123",
  "pipeline_task_id": "task-123"
}
```

If transcription fails, the endpoint returns a handled failure:

```json
{
  "status": "failed",
  "model": "granite",
  "error_code": "transcription_failed",
  "idempotency_key": "request-123",
  "pipeline_task_id": "task-123"
}
```

## Adding A Model

Model selection is explicit in `run_transcription()`. To add another model:

1. Add its public identifier to `ModelType`.
2. Add a warm Modal model class with a `transcribe(audio_url: str) -> str`
   method, following `src/socialguard_models/granite.py`.
3. Add a `case` for its identifier in `run_transcription()` that starts the worker
   with `NewModel().transcribe.spawn(audio_url)`. Waiting, cancellation, and
   response handling are shared across models.

Keep backend-specific dependencies in its Modal image rather than the local
project environment. The shared API contract should continue returning only
text unless the service requirements change.
