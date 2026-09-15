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

## Project layout

```text
src/socialguard_models/
├── api/
│   ├── contracts.py           # Shared request metadata and queued response
│   ├── submission.py          # Shared acceptance, scheduling, and CPU dispatch
│   ├── transcription_api.py   # POST /transcription/
│   └── moderation_api.py      # POST /moderation/
├── transcription/
│   ├── contracts.py           # Transcription model IDs, tasks, and outcomes
│   ├── job.py                 # Transcription inputs, GPU dispatch, output mapping
│   ├── callbacks.py           # Transcription callback payloads
│   └── models/granite.py      # Granite GPU runtime and image
├── moderation/
│   ├── contracts.py           # Audio + transcription inputs and category scores
│   ├── callbacks.py           # Proposed moderation callback payload mapping
│   ├── job.py                 # Moderation inputs, GPU dispatch, output mapping
│   └── models/registry.py     # Model IDs mapped to GPU submission functions
├── contracts.py              # Shared audio task metadata and failure outcome
├── model_job.py              # Typed adapter boundary for any model family
├── orchestration.py          # Shared CPU worker, GPU waiting/cancellation, callbacks
├── aws.py                    # OIDC credentials and S3 presigning
├── callbacks.py              # Shared callback signing, delivery, and retries
├── idempotency.py            # Family-scoped asynchronous scheduling
├── modal_app.py              # Shared Modal app and CPU image
└── cpu_worker.py             # HTTP deployment and route registration
```

Model families own their model identifiers, input/output contracts, GPU dispatch,
and callback payload mapping. Both use the same submission and execution pipeline:

```text
family API route -> submit_model(job) -> idempotency claim
  -> process_model(job, task_id) [CPU]
     -> resolve S3 audio
     -> job.submit(audio_url) [spawn family-specific GPU model]
     -> await result with shared deadline and cancellation handling
     -> job.callback_outcome(result, task_id)
     -> signed callback with shared retries
```

`ModelJob[Result]` is a typed adapter for a serializable task, GPU submission, and
mapping the model's result (or shared failure) to its callback payload. An adapter
carries task data rather than a loaded model or AWS client. The CPU worker handles
credentials, audio access, deadlines, failure logging, and callback delivery for
every adapter. GPU dependencies stay in each model's image.

## Transcription API

`src/socialguard_models/api/transcription_api.py` defines `POST /transcription/`.
The existing `POST /` endpoint remains an alias, hidden from the API schema.
`src/socialguard_models/cpu_worker.py` serves it as a FastAPI application and
defines the shared CPU deployment. The route atomically claims each idempotency
key in a shared Modal Dict, starts CPU orchestration in a separate Modal function,
and returns immediately:

```text
HTTP endpoint
  -> shared idempotency claim (Modal Dict)
  -> shared model orchestration (CPU)
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

The spawned task calls `run_model()` in
`src/socialguard_models/orchestration.py`. That function creates a short-lived S3
URL using Modal OIDC credentials and asks the job adapter to submit GPU work.
`TranscriptionJob.submit()` selects the GPU worker with an explicit `match` on the
model. The shared pipeline waits for the result within a deadline that includes
preparation and submission time. Failures produce a shared failed outcome and
trigger best-effort cancellation if a worker was submitted.

The function logs success or failure without transcript text, task tokens, or
exception messages. It posts the terminal outcome to the URI configured by the
required `CALLBACK_URI` environment variable, which the orchestration
validates before starting any model. Failure callbacks
identify the exception type but omit its message to avoid exposing signed URLs,
credentials, internal paths, or provider response bodies. Callback delivery makes
up to three attempts for network errors, request timeouts, HTTP 408/425/429, and
server errors, with exponential backoff between attempts.

Granite runs on an L4 GPU and normalizes any FFmpeg-supported audio input to
mono 16 kHz. Inputs are limited to 512 MiB and five minutes of decoded audio.
The duration limit keeps audio embeddings and generated text within the model's
context window while bounding temporary disk, memory, and inference time.

The AWS role named by `AWS_ROLE_ARN` must trust Modal's OIDC provider and permit
`s3:GetObject` for input objects and `execute-api:Invoke` for the callback API.
The endpoint also requires Modal proxy authentication.

Create the named Modal Secret that injects the required runtime configuration
into the remote orchestration worker. For audio objects in the London region,
set `AWS_REGION` to `eu-west-2`. Setting these variables only in the shell
that runs `modal deploy` does not automatically expose them to Modal containers:

```sh
uv run modal secret create socialguard-transcription-runtime \
  --from-dotenv .env \
  --force
```

Use `--force` when intentionally replacing an existing secret. The secret must
exist in the same Modal environment used by `modal serve` or `modal deploy`.
Deployment validates the shared AWS keys. The shared CPU worker validates each
job family's callback URI before starting its model. Configure the shared
`CALLBACK_URI` in the same secret; all model families deliver to it.

Verify the Granite image imports without downloading the model or running GPU
inference:

```sh
uv run modal run src/socialguard_models/transcription/models/granite.py::verify_granite_image_imports
```

This runs a non-GPU ephemeral Modal App from the current definitions and imports
Torch, TorchAudio, and the same Transformers classes used by Granite Speech.

Preload the pinned Granite model revision into the automatically created
`socialguard-transcription-models` Modal volume:

```sh
uv run modal run src/socialguard_models/transcription/models/granite.py::download_granite_model
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
curl -X POST "${MODAL_ENDPOINT_URL%/}/transcription/" \
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

Once scheduling is recorded, an identical retry returns the original `task_id`
without scheduling another task. Retries that arrive while Modal is acknowledging
the original `spawn()` wait briefly for that scheduling result, so they are never
told that an unscheduled task is queued.
Pending scheduling claims expire after 30 seconds. A later retry recovers an expired
claim with the same `task_id`, preventing a process exit or failed state write from
blocking that idempotency key permanently. Because dispatch and state persistence are
separate remote operations, recovery may resubmit a call whose acceptance could not be
recorded; both calls retain the same logical task ID. Claims are never deleted, so an
idempotency key cannot be reassigned to a different task after partial failure.

## Moderation API

`POST /moderation/` has a separate request contract containing the same audio URI
and workflow metadata as transcription, plus a required `transcription` string.
`roblox-voice-safety-v3` is audio-only, so it accepts but does not use that
transcription:

```json
{
  "model": "roblox-voice-safety-v3",
  "audio_uri": "s3://example/audio.wav",
  "transcription": "The words spoken in the audio.",
  "idempotency_key": "request-123",
  "pipeline_task_id": "task-123",
  "task_token": "token-123"
}
```

Audio files are referenced by S3 URI, following the existing transcription input
convention. The route constructs `ModerationJob` and schedules it through the same
`submit_model()` helper and CPU worker as transcription. Registered model IDs come
from `moderation/models/registry.py`. The registry includes
`roblox-voice-safety-v3`; an unknown model returns `422`. Registered models receive
`202 Accepted`, and retries replay the accepted task ID.

### Moderation output and proposed callback

The model returns a `ModerationScores` map with all five required keys and float
values. `moderation/contracts.py` defines this as a `TypedDict`, alongside
`CompletedOutcome(scores=...)` and the shared `FailedOutcome`. The score range and
meaning are model-defined; the contract does not assume probabilities or add
thresholds or a combined verdict. These are typed domain contracts, not runtime
validation of arbitrary model output.

The proposed callback retains transcription's `taskToken` and `outcome` envelope:

```json
{
  "taskToken": "token-123",
  "outcome": {
    "type": "success",
    "moderationResult": {
      "jobId": "task-123",
      "moderationTaskId": "moderation_0123456789abcdef0123456789abcdef",
      "scores": {
        "sexual": 0.01,
        "hate_or_discrimination": 0.02,
        "harassment_or_abuse": 0.03,
        "violence_or_threats": 0.04,
        "asking_for_pii": 0.05
      }
    }
  }
}
```

`jobId` carries the input `pipeline_task_id`; `moderationTaskId` carries the queued
task ID. `moderationResult` replaces `transcriptionResult`, and `scores` replaces
the transcript string. The input audio URI and transcription are not echoed.
The queued HTTP response stays `{ "status": "queued", "task_id": "..." }`.

Failure uses the same envelope with a moderation-specific error identifier:

```json
{
  "taskToken": "token-123",
  "outcome": {
    "type": "failure",
    "error": "ModerationFailed",
    "cause": "TimeoutError"
  }
}
```

`moderation/callbacks.py` defines the `ModerationResult` type and pure
`completion_outcome()` mapping. The shared pipeline supplies the outer task token
and handles signing/delivery. This is the proposed producer contract; the callback
consumer still needs support for it.

### Roblox Voice Safety v3

`roblox-voice-safety-v3` pins
[`Roblox/voice-safety-classifier-v3`](https://huggingface.co/Roblox/voice-safety-classifier-v3)
at revision `ddb1ffb03f2a53236b6d61dd846bfc286b1d4889`. It accepts any
FFmpeg-decodable input, downloads only absolute HTTPS URLs, and normalizes to
16 kHz mono PCM16. Encoded input is limited to 512 MiB and decoded audio to
300 seconds. The classifier processes 15-second windows at a 12-second stride
(three-second overlap); the final full window is end-anchored. A five-minute
recording therefore uses 25 windows. Category scores are the maximum applicable
head score across every window:

| Returned score | Roblox heads |
| --- | --- |
| `sexual` | `SEXUAL_CONTENT`, `DATING_AND_ROMANTIC_CONTENT` |
| `hate_or_discrimination` | `DISCRIMINATORY` |
| `harassment_or_abuse` | `HARASSMENT`, `PROFANITY` |
| `violence_or_threats` | `ILLEGAL_AND_REGULATED_CONTENT` |
| `asking_for_pii` | `PRIVACY_ASKING_FOR_PII` |

The upstream names are prefixed with `ABUSE_TYPE_`. Its disruptive-audio head
is validated but intentionally not returned. These are broad model-head
semantics: `ILLEGAL_AND_REGULATED_CONTENT`, for example, is not a dedicated
violence or threat detector. `sexual` also includes dating/romantic content,
`harassment_or_abuse` includes untargeted profanity, and `asking_for_pii` detects
requests rather than general PII disclosure. The returned maxima are bounded
evidence scores, not calibrated probabilities that a recording contains a
violation. Choose thresholds using representative labeled audio, accounting for
language and recording length.

Empty audio is rejected. Nonempty clips shorter than 80 ms are padded with silence
to 80 ms; other short windows are padded to a multiple of 320 samples (20 ms) to
align the upstream feature and attention-mask paths. Padding never extends a model
input beyond 15 seconds. Mono normalization downmixes all channels, rather than
using only the first channel as the upstream WAV reader does.

The immutable GPU image contains the reviewed reference `inference.py` and
license. The exact config and weights are cached in the Modal volume
`socialguard-moderation-models`; an empty cache is populated at first inference,
or preload it explicitly:

```sh
uv run modal run src/socialguard_models/moderation/models/roblox_voice_safety.py::verify_roblox_image_imports
uv run modal run src/socialguard_models/moderation/models/roblox_voice_safety.py::download_roblox_model
```

The worker uses an L4 GPU with 16 GiB host memory, a 300-second scale-down
window, and a 660-second execution deadline. For a direct development check
that bypasses HTTP, S3 presigning, scheduling, and callbacks, run:

```sh
uv run modal run scripts/moderate.py --audio-file ./sample.wav
uv run modal run scripts/moderate.py --audio-url https://example.com/sample.wav
uv run modal run scripts/verify_roblox.py
```

The verifier uses real weights plus synthetic silence/noise to exercise short
clips, window boundaries, upstream preprocessing equivalence, and the time
budget. On an NVIDIA L4 run on 2026-09-15, model load took 22.0 seconds, warm
15- and 300-second silence inference took 0.31 and 1.42 seconds respectively,
and peak CUDA allocation was 1.29 GiB (host RSS 7.13 GiB); adapter and upstream
noise scores matched exactly. These synthetic measurements do not establish
moderation quality.

### Adding a moderation model

`ModerationJob` implements `ModelJob[ModerationScores]`. It stores only its
`ModerationTask`, so it can be serialized to the shared CPU worker. It dispatches
the resolved audio URL and transcription to the selected model, and maps the
returned scores or shared failure to the callback above. The callback URI comes
from the shared `CALLBACK_URI` environment variable.

To connect a concrete model:

1. Add a GPU runtime under `moderation/models/` accepting `(audio_url,
   transcription)` and returning the defined `ModerationScores` map.
2. Define an asynchronous submission function with this signature:
   `submit(audio_url: str, transcription: str) -> SubmittedModel[ModerationScores]`.
   It calls `await YourModel().moderate.spawn.aio(audio_url, transcription)` and
   returns the call handle immediately, leaving result waiting to the shared CPU
   pipeline.
3. Import the function in `moderation/models/registry.py` and add its public model
   ID to `MODEL_SUBMITTERS`. Register in source, not by mutating the registry in an
   HTTP request, so API and CPU containers load identical definitions.
4. Implement support for the proposed payload in the callback consumer; the
   shared runtime secret already provides `CALLBACK_URI`.

The job and API route require no changes when adding a model to the registry.

This reuses the same CPU Modal function, concurrency limits, S3 access, worker
timeout/cancellation, failure handling, scheduling, and callback delivery. Only
the GPU model and its input/output mapping differ.

Scheduling isolates identical keys across model families. Existing transcription
claims, task ID prefixes, and Modal resource names are retained.

## Testing A Model Directly

`scripts/transcribe.py` invokes the GPU worker directly, bypassing the HTTP
endpoint, idempotency, S3 presigning, and callbacks. Pass either a local file or
an absolute HTTPS URL:

```sh
uv run modal run scripts/transcribe.py --audio-file ./sample.wav
uv run modal run scripts/transcribe.py --audio-url https://example.com/sample.wav
```

The transcript is printed to stdout and the elapsed time to stderr. `modal run`
creates an ephemeral App from the current local definitions, using the current
GPU image definition and shared model cache volume. This can confirm a model
change before redeploying the endpoint.

## Adding A Model

Transcription model selection is explicit in `TranscriptionJob.submit()`. To add another model:

1. Add its public identifier to `ModelType` in `transcription/contracts.py`.
2. Add a warm Modal model class with a `transcribe(audio_url: str) -> str`
   method, following `src/socialguard_models/transcription/models/granite.py`.
3. Add a `case` for its identifier in `TranscriptionJob.submit()` that starts the
   worker with `await NewModel().transcribe.spawn.aio(audio_url)` and returns the
   submitted call. Waiting, cancellation, scheduling, and callback delivery are
   shared across all model families in `orchestration.py` and `api/submission.py`.

Keep backend-specific dependencies in its Modal image rather than the local
project environment. Transcription returns text; moderation returns its category
score map through the same execution pipeline.
