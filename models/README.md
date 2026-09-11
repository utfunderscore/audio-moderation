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
- `audio_uri` — an `s3://bucket/key` URI accessible to the worker's AWS identity;
- `callback_url` — an HTTPS webhook endpoint for completion or failure events;
- `report_id` — the caller's positive integer report identifier, echoed in the `202` response
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
The delivery worker exchanges its runtime-provided `MODAL_IDENTITY_TOKEN` through AWS
STS `AssumeRoleWithWebIdentity`, keeps the temporary credentials in memory, and
SigV4-signs the exact JSON body bytes with the STS session token. A `204` response
acknowledges delivery. A `400` payload rejection or `403` authentication rejection stops
local retries; temporary failures receive up to three attempts with one- and two-second
backoff. Modal retries redeliver the exact persisted bytes after temporary callback
exhaustion.

The deployed gateway uses a bounded Modal Dict ledger. Its idempotency and recovery
records expire after seven days of inactivity, and a ledger write cannot be transactional
with spawning a Modal worker. Concurrent identical requests use a bounded handoff:
they return `202` only after one request has spawned work or the ledger confirms it was
dispatched. A temporary `503` is safe to retry with the same idempotency key. Use a
transactional external database before requiring longer retention or stronger outbox
guarantees. The worker extracts the bucket and object key from `audio_uri` and downloads
it with boto3. IAM determines which buckets are accessible. S3 downloads are limited to
25 MiB compressed bytes and 60 seconds decoded duration. The worker allows 90 seconds
for download, 180 seconds for GPU inference, and 420 seconds total; the GPU gets a
600-second cold-start budget.
Callback URL dereferencing rejects non-HTTPS, non-443, credential-bearing,
fragment-bearing, and private-network destinations.

### Deploy prerequisites

Create the `socialguard-gateway-api` Modal Secret containing
`SOCIALGUARD_GATEWAY_API_TOKEN`. Both S3 retrieval and callback delivery use temporary
credentials obtained from Modal's runtime OIDC identity; no permanent AWS key or S3
Modal Secret is required. The callback URL, role ARN, region, and SigV4 service are
non-secret worker configuration. `SOCIALGUARD_S3_ENDPOINT_URL` remains optional for an
S3-compatible endpoint. See [`.env.example`](.env.example) for the complete environment
shape. Prefetch the pinned model cache before deployment. The deployment entrypoint is
`socialguard_models.deployments.granite`; it has a CPU ASGI gateway, CPU orchestrator,
and lifecycle-loaded L40S worker with one active inference per container.

### Deploy

After completing the prerequisites, deploy the gateway from any working directory:

```bash
./scripts/deploy-modal.sh
```

The script verifies the lockfile, formatting, linting, types, and tests before invoking
`modal deploy`. It does not prefetch the model; run the prefetch command above first.

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
```bash
curl -X PUT \
  -H "Authorization: Bearer $MODAL_PROXY_TOKEN_ID.$MODAL_PROXY_TOKEN_SECRET" \
  -H "Content-Type: application/json" \
  --data '{"model":"granite","audio_uri":"s3://audio-bucket/evaluations/42/audio.wav","idempotency_key":"transcription-42","pipeline_task_id":"42","task_token":"<external-task-token>"}' \
  "<transcription-endpoint-url>"
```

## AWS OIDC

Modal Functions receive a short-lived `MODAL_IDENTITY_TOKEN`. Configure AWS to
trust Modal's OIDC provider, `https://oidc.modal.com`, then restrict the role's
trust policy to this Modal workspace, app, or function. Store the role ARN and
AWS region in a Modal Secret:

```bash
uv run modal secret create aws-oidc-role \
  AWS_ROLE_ARN="arn:aws:iam::<account-id>:role/<role-name>" \
  AWS_REGION="us-east-1"
```

The transcription API and CPU coordinator receive that Secret, then use
`create_aws_client` to exchange the Modal identity token for short-lived AWS
credentials:

```python
from socialguard_models.aws_oidc import create_aws_client

s3 = create_aws_client("s3")
```

See [Modal's OIDC guide](https://modal.com/docs/guide/oidc-integration) for
the required AWS OIDC provider and IAM role trust policy.

## Granite Worker

`src/socialguard_models/transcriptions/models/granite_model.py` implements transcription with
[IBM Granite Speech 4.1 2B](https://huggingface.co/ibm-granite/granite-speech-4.1-2b).
The model supports English, French, German, Spanish, Portuguese, and Japanese.
Its released configuration declares Transformers 4.57.6, which the Modal image
pins. It is prompted with `<|audio|>transcribe the speech with proper
punctuation and capitalization.`.

The API validates the external `audio_uri` with `HeadObject`, then uses that
same temporary S3 client to create a one-hour `get_object` presigned URL for
the exact bucket and key. It passes that bearer URL only through the internal
Modal RPC as `TranscriptionJob(download_url="...")`; the GPU worker streams it
directly to its existing temporary file and has no AWS secret, OIDC exchange,
or S3 client. The API role needs `s3:GetObject` for the input (which also
authorizes the `HeadObject` operation); its IAM trust policy can be restricted
to the CPU API and coordinator identities rather than the GPU worker. The role
also needs `lambda:InvokeFunction` on `audio-moderation-dev-task-callback`, and
its trust policy must allow the `coordinate_transcription` function identity.

The URL's actual usable lifetime is the earlier of one hour and the temporary
AWS session's expiration, so queue delay also consumes it. An expired or HTTP-
failed download fails the queued task without exposing the URL or falling back
to GPU AWS credentials. Submit a new request with a new idempotency key to
retry; an existing key continues to return its original task ID.

Input must already be mono PCM WAV at 16 kHz; no conversion is performed. IBM's
base model tree does not provide VAD tooling, so the worker uses the external,
pinned [Silero VAD 6.2.1](https://pypi.org/project/silero-vad/6.2.1/) package.
It scans the complete CPU waveform with Silero's
[`load_silero_vad` and `get_speech_timestamps`](https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/utils_vad.py),
then sends only detected speech to Granite in order. The cached VAD instance is
reset by `get_speech_timestamps` for each recording.

Silero receives a one-dimensional CPU float waveform at 16 kHz with threshold
0.5, minimum speech duration 100 ms (lower than Silero's 250 ms default to
retain short utterances), minimum silence 500 ms, 100 ms speech padding, and a
30-second maximum speech duration, returning sample coordinates
(`return_seconds=False`). Silero accounts for padding in its maximum; Granite
inputs are still defensively capped at 30 seconds if VAD returns a longer
range. This may hard-split uninterrupted speech when no suitable silence exists;
quiet speech can also be missed by VAD.
The complete recording is held in CPU memory for VAD (about 230 MB per hour of
16 kHz mono float32 audio), unlike the previous block reader. The worker does
not provide timestamps or speaker labels.

The Modal image contains the ML dependencies on Python 3.12, independently of
the local project's Python version. Model weights are stored persistently in the
named `socialguard-model-weights` Modal Volume at
`/models/granite-speech-4.1-2b/de575db64086f84fdc79da4932d1076e965bc546` and
mounted read-only by the worker. Download the pinned weights once before using
or deploying the worker:

```bash
uv run modal run src/socialguard_models/transcriptions/models/granite_model.py::download_granite_model
```

The CPU-only download function commits the Volume only after a successful
snapshot. Workers load only local files and report this command if the ready
marker is missing; they never download from Hugging Face. To update Granite,
set `MODEL_REVISION` to the desired immutable Hugging Face commit, run the
download command to create its new Volume directory, then deploy the worker.
Silence returns empty text without loading Granite. Empty audio, decoding
failures, and generation reaching the token limit raise errors rather than
returning a potentially incomplete result.

Resource limits favor low-cost testing: at most one L4 container, no minimum
warm pool or spare containers, a 2-second idle scaledown window, and a 5-minute
per-job timeout. L4 supports the worker's BF16 inference at a lower listed GPU
rate than A10. Expect more cold starts for model loading and queued requests
rather than parallel GPU workers. Increase the timeout for long recordings;
these settings are not a total spending cap.

The worker and HTTP API are separate Modal apps. Their internal RPC job
contract changes together, so coordinate deployment and do not run a new API
with an old worker (or the reverse). Deploy the worker first so the API can
resolve its function by name, then deploy the API:

```bash
uv run modal deploy src/socialguard_models/transcriptions/models/granite_model.py
uv run modal deploy src/socialguard_models/transcriptions/transcription_api.py
```

The API asynchronously spawns `coordinate_transcription` in the CPU app and
returns the coordinator's Modal function-call ID as `task_id`. The coordinator
awaits the GPU function through Modal's async RPC, delivers a terminal callback,
then returns the normalized transcription result or propagates the GPU error. Requests with the same
`idempotency_key` return the original task ID. Task IDs issued before this change
still identify their original GPU calls.

The coordinator requests 0.125 physical CPU cores and 256 MiB RAM, scales to zero,
and supports up to 20 concurrent async inputs per container. Its execution timeout
is 10 minutes, including GPU queueing, inference, and callback delivery. Increase this
budget if queues can exceed it; a coordinator timeout does not establish that
the GPU task has failed. Application-level coordinator retries are disabled to
avoid automatically repeating inference after an error.

`transcriptions/coordinator.py` is imported by the API deployment entry point, so
both CPU functions deploy together with the existing command. It receives the
presigned download URL, model, idempotency key, required string `pipeline_task_id`,
and required nonempty `task_token`. The tracking identifiers remain internal;
the callback uses the external task token.

After inference, the coordinator exchanges its OIDC token for fresh AWS credentials
and invokes `audio-moderation-dev-task-callback` synchronously through
`TaskCallbackClient`. The callback body contains `taskToken` and one of:

```json
{"type":"success","result":{"transcript":"The transcribed text."}}
```

```json
{"type":"failure","error":"TranscriptionFailed","cause":"The transcription service could not process the audio."}
```

Silence is a successful empty transcript. Inference exceptions produce the fixed
failure message rather than raw exception details. Delivery runs in a background
thread so boto3 and retry sleeps do not block other coordinator inputs. Transient
delivery errors receive up to five application-level attempts with exponential
backoff and jitter; acceptance requires HTTP 204 from the callback handler.
Delivery errors propagate without re-running inference or sending a contradictory
failure outcome after success. If failure delivery also fails, the delivery error
retains the inference error as its exception context.

Retries are in-process, not durable across coordinator termination or timeout.
The receiver must tolerate repeated task tokens after ambiguous delivery failures.
Reusing an idempotency key returns its original task ID and does not register a new
callback token or trigger callback re-delivery.

## Roblox Voice Safety Worker

`src/socialguard_models/voice_safety/models/roblox/roblox_model.py` implements
voice-safety classification with
[Roblox voice-safety-classifier-v3](https://huggingface.co/Roblox/voice-safety-classifier-v3).
It accepts a presigned download URL internally and requires a 16 kHz PCM WAV file;
multichannel input uses its first channel. Recordings may be up to 10 minutes.
The worker processes them sequentially in 15-second windows with 3 seconds of
overlap, avoiding both Roblox's 30-second loader truncation and a large GPU batch.

The worker returns independent probabilities normalized to the shared `sexual`,
`hate_or_discrimination`, `harassment_or_abuse`, `violence_or_threats`, and
`asking_for_pii` categories. Combined categories and results across windows use
the maximum upstream score. Language probabilities are averaged across windows,
weighted by their sample counts.
The upstream disruptive-audio score is intentionally omitted, and the model's
30-way language probabilities are returned unchanged.

The immutable model snapshot includes Roblox's self-contained inference module
and is stored in the shared `socialguard-model-weights` Modal Volume at
`/models/voice-safety-classifier-v3/ddb1ffb03f2a53236b6d61dd846bfc286b1d4889`.
Download it once before using or deploying the worker:

```bash
uv run modal run src/socialguard_models/voice_safety/models/roblox/roblox_model.py::download_roblox_model
uv run modal deploy src/socialguard_models/voice_safety/models/roblox/roblox_model.py
```

Workers load only the pinned local snapshot and never fetch model files during
inference. The worker uses one L4 GPU, scales to zero after two idle seconds, and
allows at most one container for low-cost initial operation.
