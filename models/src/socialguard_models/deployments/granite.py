"""Modal deployment for the asynchronous Granite OpenAPI gateway.

This is a bounded MVP: Modal Dict records expire after seven days of inactivity and
cannot atomically combine ledger writes with function spawning. Identical retries repair
``dispatch_pending`` records; use a transactional database before promising longer
retention or stronger outbox guarantees.
"""

import logging
import os
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import TYPE_CHECKING, cast

if TYPE_CHECKING:
    from transformers import Processor, SpeechModel

import modal

from socialguard_models.api.app import create_gateway_app
from socialguard_models.api.auth import GatewaySettings
from socialguard_models.api.callbacks import (
    CallbackAuthenticationError,
    CallbackContractError,
    CallbackDeliveryError,
    SigV4CallbackSigner,
    deliver_with_retries,
    terminal_body,
)
from socialguard_models.api.networking import (
    NetworkPolicy,
    validate_url,
)
from socialguard_models.api.observability import log_event
from socialguard_models.api.submission import (
    LedgerSubmitter,
    SubmissionCommand,
    SubmissionDispatcher,
    SubmissionLedger,
    SubmissionRecord,
)
from socialguard_models.deployments.aws_credentials import (
    TemporaryAwsCredentials,
    TemporaryAwsCredentialsError,
    assume_role_with_modal_identity,
)
from socialguard_models.deployments.granite_resources import (
    CACHE_DIRECTORY,
    CPU_CORES,
    GPU,
    HOST_MEMORY_MIB,
    MAX_ACTIVE_INPUTS,
    MODEL_CACHE,
    MODEL_ID,
    MODEL_REVISION,
    VENDOR_MAX_NEW_TOKENS,
)
from socialguard_models.deployments.granite_resources import (
    IMAGE as GRANITE_IMAGE,
)
from socialguard_models.deployments.s3_audio import (
    AUDIO_EMPTY_MESSAGE,
    AudioRejectedError,
    AudioRetrievalError,
    download_audio,
)

APP = modal.App("socialguard-asr-gateway")
# Modal CLI deployment defaults to a module-level variable named ``app``.
app = APP
API_IMAGE = modal.Image.debian_slim(python_version="3.12").uv_sync(
    uv_project_dir=".", frozen=True, extra_options="--no-dev"
)
GATEWAY_SECRET = modal.Secret.from_name(
    "socialguard-gateway-api", required_keys=["SOCIALGUARD_GATEWAY_API_TOKEN"]
)
CALLBACK_ENV: dict[str, str] = {
    "ASR_CALLBACK_URL": (
        "https://e4rytge57sepkksrukjcy3p7ze0ezqnl.lambda-url.eu-west-2.on.aws/"
    ),
    "ASR_CALLBACK_ROLE_ARN": (
        "arn:aws:iam::967883357915:role/socialguard-dev-modal-asr-callback-sender"
    ),
    "AWS_REGION": "eu-west-2",
    "AWS_SIGV4_SERVICE": "lambda",
}
WORKER_ENV = dict(CALLBACK_ENV)
if endpoint_url := os.environ.get("SOCIALGUARD_S3_ENDPOINT_URL"):
    WORKER_ENV["SOCIALGUARD_S3_ENDPOINT_URL"] = endpoint_url
JOB_LEDGER = modal.Dict.from_name(
    "socialguard-asr-gateway-jobs", create_if_missing=True
)
GATEWAY_MAX_CONTAINERS = 2
WORKER_MAX_CONTAINERS = 2
MAX_AUDIO_SECONDS = 60
TARGET_SAMPLE_RATE = 16_000
AUDIO_TOO_LONG_MESSAGE = "Audio exceeds the 60-second duration limit."
GPU_TIMEOUT_SECONDS = 180
WORKER_TIMEOUT_SECONDS = 420
GPU_STARTUP_TIMEOUT_SECONDS = 600
WORKER_RETRIES = modal.Retries(
    max_retries=2, initial_delay=1.0, backoff_coefficient=2.0, max_delay=2.0
)


class _ModalLedger(SubmissionLedger):
    """Adapt Modal Dict's atomic create-if-absent operation to the API seam."""

    async def get(self, key: str) -> SubmissionRecord | None:
        value = await JOB_LEDGER.get.aio(key)
        return value if isinstance(value, SubmissionRecord) else None

    async def put_if_absent(self, key: str, record: SubmissionRecord) -> bool:
        return await JOB_LEDGER.put.aio(key, record, skip_if_exists=True)

    async def set_state(self, key: str, state: str) -> None:
        record = await self.get(key)
        if record is not None:
            await JOB_LEDGER.put.aio(
                key,
                SubmissionRecord(
                    transcription_id=record.transcription_id,
                    fingerprint=record.fingerprint,
                    state=state,
                    command=record.command,
                ),
            )

    async def claim_dispatch(self, key: str) -> bool:
        """Atomically own dispatch so simultaneous retries start one worker."""
        return await JOB_LEDGER.put.aio(
            f"dispatch-claim:{key}", "claimed", skip_if_exists=True
        )

    async def release_dispatch_claim(self, key: str) -> None:
        """Allow a retry to repair work when spawning was not accepted."""
        await JOB_LEDGER.pop.aio(f"dispatch-claim:{key}", None)


class _ModalDispatcher(SubmissionDispatcher):
    """Dispatch CPU work without holding the ASGI request open for inference."""

    async def dispatch(
        self,
        ledger_key: str,
        transcription_id: str,
        command: SubmissionCommand,
    ) -> None:
        await process_transcription.spawn.aio(ledger_key, transcription_id, command)


def _failure_event(
    transcription_id: str, command: SubmissionCommand, code: str, message: str
) -> dict[str, object]:
    """Construct one schema-compatible terminal failure event."""
    return {
        "type": "transcription.failed",
        "id": transcription_id,
        "report_id": command.report_id,
        "model": command.model.value,
        "error": {"code": code, "message": message},
    }


@APP.function(  # pyright: ignore[reportUnknownMemberType]
    image=API_IMAGE,
    env=cast("dict[str, str | None]", WORKER_ENV),
    timeout=WORKER_TIMEOUT_SECONDS,
    retries=WORKER_RETRIES,
    max_containers=WORKER_MAX_CONTAINERS,
)
async def process_transcription(
    ledger_key: str,
    transcription_id: str,
    command: SubmissionCommand,
) -> None:
    """Repair dispatch state, then persist and deliver one terminal event."""
    import asyncio

    log_event(
        "worker_job_started",
        transcription_id=transcription_id,
        report_id=command.report_id,
        model=command.model.value,
        audio_uri=command.audio_uri,
    )
    await _ModalLedger().set_state(ledger_key, "dispatched")
    terminal_key = f"terminal:{transcription_id}"
    existing = await JOB_LEDGER.get.aio(terminal_key)
    if isinstance(existing, bytes):
        log_event(
            "terminal_replay",
            transcription_id=transcription_id,
            report_id=command.report_id,
        )
        credentials = await asyncio.to_thread(_assume_aws_credentials)
        await _deliver_callback(command.callback_url, existing, credentials)
        return
    credentials = await asyncio.to_thread(_assume_aws_credentials)
    try:
        audio = await download_audio(command.audio_uri, credentials)
        text = cast(
            "str",
            await GraniteModel().transcribe_bytes.remote.aio(audio),  # pyright: ignore[reportUnknownMemberType,reportCallIssue]
        )
        if text:
            event: dict[str, object] = {
                "type": "transcription.completed",
                "id": transcription_id,
                "report_id": command.report_id,
                "model": command.model.value,
                "data": {"text": text},
            }
        else:
            event = _failure_event(
                transcription_id,
                command,
                "transcription_failed",
                "Transcription could not be completed.",
            )
    except AudioRejectedError as exc:
        log_event(
            "audio_rejected",
            level=logging.WARNING,
            transcription_id=transcription_id,
            reason=str(exc),
        )
        event = _failure_event(transcription_id, command, "audio_rejected", str(exc))
    except AudioRetrievalError:
        log_event(
            "audio_retrieval_failed",
            level=logging.WARNING,
            transcription_id=transcription_id,
            audio_uri=command.audio_uri,
        )
        event = _failure_event(
            transcription_id,
            command,
            "audio_retrieval_failed",
            "Audio could not be retrieved.",
        )
    except Exception as exc:  # noqa: BLE001 - terminal events must cover unexpected inference errors.
        log_event(
            "transcription_unexpectedly_failed",
            level=logging.ERROR,
            transcription_id=transcription_id,
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        event = _failure_event(
            transcription_id,
            command,
            "transcription_failed",
            "Transcription could not be completed.",
        )
    body = terminal_body(event)
    await JOB_LEDGER.put.aio(terminal_key, body, skip_if_exists=True)
    winning = await JOB_LEDGER.get.aio(terminal_key)
    if not isinstance(winning, bytes):
        message = "terminal event persistence failed"
        raise TypeError(message)
    await _deliver_callback(command.callback_url, winning, credentials)
    log_event(
        "worker_job_finished",
        transcription_id=transcription_id,
        report_id=command.report_id,
        outcome=event["type"],
    )
    await asyncio.sleep(0)


async def _deliver_callback(
    url: str, body: bytes, credentials: TemporaryAwsCredentials
) -> None:
    """Send a bounded callback without logging request data or secrets."""
    import httpx

    validate_url(url, NetworkPolicy())

    class Sender:
        async def send(self, url: str, body: bytes, headers: dict[str, str]) -> int:
            timeout = httpx.Timeout(timeout=20.0, connect=5.0, read=10.0)
            async with (
                httpx.AsyncClient(follow_redirects=False, timeout=timeout) as client,
                client.stream("POST", url, content=body, headers=headers) as response,
            ):
                return response.status_code

    configured_url = os.environ["ASR_CALLBACK_URL"]
    if url != configured_url:
        log_event(
            "callback_contract_rejected",
            level=logging.ERROR,
            detail="Callback URL does not match the configured AWS target.",
        )
        return
    try:
        signer = _callback_signer(credentials)
        await deliver_with_retries(Sender(), url, body, signer, sleep=_sleep)
    except CallbackContractError:
        log_event(
            "callback_contract_rejected",
            level=logging.ERROR,
            detail="Callback payload was rejected and will not be retried locally.",
            exc_info=True,
        )
        return
    except CallbackAuthenticationError:
        log_event(
            "callback_authentication_rejected",
            level=logging.ERROR,
            detail="OIDC, IAM, or SigV4 callback authentication failed.",
            exc_info=True,
        )
        return
    except CallbackDeliveryError:
        log_event(
            "callback_exhausted",
            level=logging.ERROR,
            detail="Callback was not acknowledged; Modal will redeliver this job.",
            exc_info=True,
        )
        raise


def _assume_aws_credentials() -> TemporaryAwsCredentials:
    """Exchange the runtime Modal identity for in-memory AWS session credentials."""
    if not os.environ.get("MODAL_IDENTITY_TOKEN"):
        message = "MODAL_IDENTITY_TOKEN is unavailable"
        raise CallbackAuthenticationError(message)
    try:
        return assume_role_with_modal_identity(
            role_arn=os.environ["ASR_CALLBACK_ROLE_ARN"],
            region=os.environ["AWS_REGION"],
        )
    except TemporaryAwsCredentialsError as exc:
        raise CallbackAuthenticationError(str(exc)) from exc


def _callback_signer(credentials: TemporaryAwsCredentials) -> SigV4CallbackSigner:
    """Build a callback signer from the AWS session used for audio retrieval."""
    return SigV4CallbackSigner(
        access_key_id=credentials.access_key_id,
        secret_access_key=credentials.secret_access_key,
        session_token=credentials.session_token,
        region=os.environ["AWS_REGION"],
        service=os.environ["AWS_SIGV4_SERVICE"],
    )


async def _sleep(seconds: float) -> None:
    """Isolate retry delay for tests and preserve async worker scheduling."""
    import asyncio

    await asyncio.sleep(seconds)


@APP.cls(  # pyright: ignore[reportUnknownMemberType]
    image=GRANITE_IMAGE,
    gpu=GPU,
    cpu=CPU_CORES,
    memory=HOST_MEMORY_MIB,
    timeout=GPU_TIMEOUT_SECONDS,
    startup_timeout=GPU_STARTUP_TIMEOUT_SECONDS,
    max_containers=1,
    min_containers=0,
    volumes={CACHE_DIRECTORY: MODEL_CACHE.read_only()},
)
@modal.concurrent(max_inputs=MAX_ACTIVE_INPUTS)  # pyright: ignore[reportUnknownMemberType]
class GraniteModel:
    """Lifecycle-loaded Granite inference worker with one GPU input per container."""

    processor: "Processor"
    model: "SpeechModel"

    @modal.enter()  # pyright: ignore[reportUnknownMemberType]
    def load(self) -> None:
        """Load only the immutable pre-fetched snapshot; never download on requests."""
        import torch
        from huggingface_hub import snapshot_download
        from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

        snapshot = Path(
            snapshot_download(
                repo_id=MODEL_ID,
                revision=MODEL_REVISION,
                cache_dir=CACHE_DIRECTORY,
                local_files_only=True,
            )
        )
        self.processor = AutoProcessor.from_pretrained(snapshot, local_files_only=True)
        self.model = (
            AutoModelForSpeechSeq2Seq.from_pretrained(
                snapshot, local_files_only=True, torch_dtype=torch.bfloat16
            )
            .eval()
            .to("cuda")
        )

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def transcribe_bytes(self, audio_bytes: bytes) -> str:
        """Normalize uploaded audio then generate deterministic Granite output."""
        import torch
        import torchaudio

        with NamedTemporaryFile(suffix=".audio") as handle:
            handle.write(audio_bytes)
            handle.flush()
            metadata = torchaudio.info(handle.name)  # pyright: ignore[reportUnknownMemberType]
            if metadata.num_frames <= 0 or metadata.sample_rate <= 0:
                raise AudioRejectedError(AUDIO_EMPTY_MESSAGE)
            if metadata.num_frames > MAX_AUDIO_SECONDS * metadata.sample_rate:
                raise AudioRejectedError(AUDIO_TOO_LONG_MESSAGE)
            waveform, sample_rate = torchaudio.load(
                handle.name,
                normalize=True,
                num_frames=metadata.num_frames + 1,
            )
        if waveform.shape[0] > 1:
            waveform = waveform.mean(dim=0, keepdim=True)  # pyright: ignore[reportUnknownMemberType]
        if sample_rate != TARGET_SAMPLE_RATE:
            waveform = torchaudio.functional.resample(  # pyright: ignore[reportUnknownMemberType]
                waveform, sample_rate, TARGET_SAMPLE_RATE
            )
        if waveform.shape[-1] > MAX_AUDIO_SECONDS * TARGET_SAMPLE_RATE:
            raise AudioRejectedError(AUDIO_TOO_LONG_MESSAGE)
        prompt = self.processor.tokenizer.apply_chat_template(
            [
                {
                    "role": "user",
                    "content": (
                        "<|audio|>can you transcribe the speech into a written format?"
                    ),
                }
            ],
            tokenize=False,
            add_generation_prompt=True,
        )
        inputs = self.processor(
            prompt, waveform, device="cuda", return_tensors="pt"
        ).to("cuda")
        with torch.inference_mode():
            output = self.model.generate(
                **inputs,
                max_new_tokens=VENDOR_MAX_NEW_TOKENS,
                do_sample=False,
                num_beams=1,
            )
        generated = output[0, inputs["input_ids"].shape[-1] :].unsqueeze(0)
        return self.processor.tokenizer.batch_decode(
            generated, add_special_tokens=False, skip_special_tokens=True
        )[0]


@APP.function(  # pyright: ignore[reportUnknownMemberType]
    image=API_IMAGE,
    secrets=[GATEWAY_SECRET],
    timeout=30,
    max_containers=GATEWAY_MAX_CONTAINERS,
    min_containers=0,
)
@modal.concurrent(max_inputs=20, target_inputs=10)  # pyright: ignore[reportUnknownMemberType]
@modal.asgi_app()  # pyright: ignore[reportUnknownMemberType]
def gateway() -> object:
    """Return the authenticated OpenAPI gateway; no GPU is allocated here."""
    return create_gateway_app(
        LedgerSubmitter(_ModalLedger(), _ModalDispatcher()),
        GatewaySettings.from_environment(),
    )
