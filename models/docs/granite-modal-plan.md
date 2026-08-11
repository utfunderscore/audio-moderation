# Granite 4.0 Speech — First Modal Deployment

## Target

Deploy [`ibm-granite/granite-4.0-1b-speech`](https://huggingface.co/ibm-granite/granite-4.0-1b-speech)
as the first worker behind the generated asynchronous gateway contract.

> **Superseded transport design:** the synchronous multipart endpoint described in
> earlier revisions of this document is obsolete. The authoritative public contract is
> [`openapi/gateway.openapi.json`](../openapi/gateway.openapi.json):
> `POST /v1/transcriptions` accepts an HTTPS presigned audio URL and callback URL,
> responds `202`, and delivers a signed terminal callback.

## Pinned model facts

- Model ID: `ibm-granite/granite-4.0-1b-speech`
- Initial revision: `bd87ab862416353633ea431fe49b1614003623c5`
- License: Apache-2.0
- Runtime: native Hugging Face Transformers implementation
- Transformers baseline: `4.54.0`; IBM documents `>=4.52.1`
- Repository size: approximately 6.35 GB
- Parameters: approximately 2.31 billion BF16 parameters
- Input expected by the processor: mono, 16 kHz waveform
- Supported ASR languages: English, French, German, Spanish, Portuguese, Japanese

The revision is immutable deployment configuration. Never serve mutable `main`.

## MVP decisions

- Use `AutoProcessor` and `AutoModelForSpeechSeq2Seq`, not vLLM.
- Use BF16 on one NVIDIA GPU.
- Benchmark L40S first; select a smaller GPU only after measuring peak memory and latency.
- Allow one active inference per container.
- Limit decoded audio to 60 seconds initially.
- Use deterministic generation: `max_new_tokens=200`, `do_sample=False`, `num_beams=1`.
- Advertise JSON responses only.
- Advertise `language=False`: the model is multilingual, but the vendor example does not define a language-hint parameter.
- Advertise `prompt=False`: Granite keyword biasing is not equivalent to OpenAI's generic prompt field.
- Defer translation, keyword biasing, timestamps, diarization, streaming, and long-form segmentation.

## Implementation sequence

### 1. Dependency and GPU spike

Add a `backend-granite` uv dependency group containing:

- `transformers==4.54.0`
- a matched `torch` and `torchaudio` pair
- `soundfile`
- `huggingface-hub`
- `accelerate` only if the final loader uses `device_map`

Build a minimal Modal image and run one remote transcription before writing the full
adapter. This determines the exact Torch/CUDA pins and confirms BF16 support.

**Exit:** the pinned model loads from its immutable revision and transcribes IBM's
`multilingual_sample.wav` on the selected GPU.

### 2. Granite adapter

Add `src/socialguard_models/backends/granite.py` implementing `Transcriber`.

Responsibilities:

- expose immutable `ModelInfo`;
- load the processor and model once;
- decode, downmix, and resample uploads to mono 16 kHz;
- build the prompt using the tokenizer's chat template;
- run deterministic generation under `torch.inference_mode()`;
- strip input tokens before decoding output;
- return only `Transcript(text=...)`;
- never download weights inside `transcribe()`.

Keep Transformers and Torch types inside this module. Use narrow local protocols or
stubs rather than weakening project-wide basedpyright settings.

**Exit:** adapter contract tests pass against a locally cached model or remote GPU test.

### 3. Shared FastAPI endpoint

Implement the existing API plan using a typed fake before connecting Granite:

- bounded multipart upload spooling;
- model ID and alias validation;
- JSON-only response validation;
- bearer authentication;
- typed errors and request IDs;
- `/healthz`, `/readyz`, and `/v1/models`;
- synchronous adapter execution outside the ASGI event loop.

Initial model identifiers:

```text
canonical: ibm-granite/granite-4.0-1b-speech
alias:     granite-4.0-1b-speech
```

**Exit:** API integration tests pass without installing the Granite dependency group.

### 4. Modal deployment

Add `src/socialguard_models/deployments/granite.py` with:

- one `modal.App`;
- Python 3.12 image synced from `uv.lock` with `backend-granite`;
- `ffmpeg` installed explicitly;
- a named Hugging Face cache Volume;
- an idempotent prefetch function for the pinned revision;
- `@modal.enter()` model loading with `local_files_only=True`;
- `@modal.asgi_app()` returning the shared FastAPI application;
- `@modal.concurrent(max_inputs=1)`;
- bounded containers and a zero-container development minimum.

The prefetch function is the only cache writer. Serving must fail at startup if the
pinned snapshot is missing instead of downloading during a request.

**Exit:** an ephemeral Modal URL passes an authenticated multipart transcription test.

### 5. Staging and benchmark

Test representative 5-second, 30-second, and 60-second files. Record:

- image build and cold-start time;
- model load time;
- transcription latency and real-time factor;
- peak GPU memory;
- p50 and p95 latency;
- malformed audio, timeout, and OOM behavior.

Promote only after the exact HTTP contract, pinned revision, GPU choice, and cost limit
are recorded in the README.

## Test fixtures

Start with no invented golden transcript. Capture and review expected output from the
pinned model/image combination.

Required fixtures:

- IBM's pinned `multilingual_sample.wav` with its SHA-256;
- valid mono 16 kHz audio;
- stereo or 44.1/48 kHz audio proving normalization;
- silence, empty, and corrupt audio;
- one 60-second boundary case.

Add language-specific quality fixtures only after the basic deployment works.

## Validation gates

```bash
uv lock --check
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
```

Then run one paid remote GPU smoke test. Do not begin concurrency or GPU optimization
until that result is correct.

## Main risks

1. Torch, torchaudio, CUDA, and Transformers compatibility must be proven in the Modal image.
2. The model can hallucinate; silence and malformed-audio tests are required.
3. Audio over 60 seconds needs VAD segmentation and boundary reconciliation, deferred from MVP.
4. Generated output can exceed 200 tokens; the duration/output limits must be benchmarked together.
5. Generic `prompt` and `language` API fields must remain rejected until their semantics are implemented and tested.
