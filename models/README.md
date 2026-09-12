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
defines the shared CPU deployment. The route atomically claims each idempotency
key in a shared Modal Dict, starts CPU orchestration in a separate Modal function,
and returns immediately:

```text
HTTP endpoint
  -> shared idempotency claim (Modal Dict)
  -> spawned transcription control plane (CPU)
  -> selected transcription worker (GPU)
  -> completion callback API
```

The CPU deployment uses one container (`max_containers=1`) with up to 32 concurrent
requests (`modal.concurrent(max_inputs=32)`). Requests only claim and schedule work,
so GPU inference does not occupy an HTTP request. The idempotency entry is retained
by Modal for up to seven days of inactivity. The CPU container scales to zero when
idle. Adjust `MAX_CONCURRENT_REQUESTS` in `cpu_worker.py` to tune concurrency.

Each spawned orchestration worker also handles up to 32 tasks concurrently. It uses
Modal's asynchronous remote-call APIs while waiting for GPU work, so those waits do
not occupy one thread per transcription.

The spawned task calls `run_transcription()` in
`src/socialguard_models/transcription.py`. That function creates a short-lived S3
URL using Modal OIDC credentials, selects the GPU worker with an explicit `match`
on the model, submits it with `spawn()`, and waits for its result within a deadline
that includes preparation and submission time. Failures produce a failed outcome and
trigger best-effort cancellation if a worker was submitted.

The function logs success or failure without transcript text, task tokens, or
exception messages. It posts the terminal outcome to the URI configured by the
required `TRANSCRIPTION_CALLBACK_URI` environment variable, which each orchestration
validates before starting transcription. Failure callbacks
identify the exception type but omit its message to avoid exposing signed URLs,
credentials, internal paths, or provider response bodies. Callback delivery makes
up to three attempts for network errors, request timeouts, HTTP 408/425/429, and
server errors, with exponential backoff between attempts.

Granite runs on an L4 GPU and normalizes any FFmpeg-supported audio input to
mono 16 kHz. Inputs are limited to 512 MiB and five minutes of decoded audio.
The duration limit keeps audio embeddings and generated text within the model's
context window while bounding temporary disk, memory, and inference time.

The AWS role named by `AWS_ROLE_ARN` must trust Modal's OIDC provider and permit
`s3:GetObject` for input objects. The endpoint also requires Modal proxy
authentication.

Create the named Modal Secret that injects the required runtime configuration
into the remote orchestration worker. Setting these variables only in the shell
that runs `modal deploy` does not automatically expose them to Modal containers:

```sh
uv run modal secret create socialguard-transcription-runtime \
  TRANSCRIPTION_CALLBACK_URI="${TRANSCRIPTION_CALLBACK_URI:?required}" \
  AWS_ROLE_ARN="${AWS_ROLE_ARN:?required}"
```

Use `--force` when intentionally replacing an existing secret. The secret must
exist in the same Modal environment used by `modal serve` or `modal deploy`.
Deployment validates that both keys exist before starting the application.

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

The endpoint returns `202 Accepted` as soon as work has been scheduled:

```json
{
  "status": "queued",
  "task_id": "transcription_0123456789abcdef0123456789abcdef"
}
```

An identical retry returns the original `task_id` without scheduling another task.
Retries that arrive while Modal is acknowledging the original `spawn()` wait briefly
for that scheduling result, so they are never told that an unscheduled task is queued.

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
