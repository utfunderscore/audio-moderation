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
    from transformers import (  # pyright: ignore[reportMissingModuleSource]
        Processor,
        SpeechModel,
    )

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
from socialguard_models.api.observability import log_event, redacted_url
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
API_IMAGE = (
    modal.Image.debian_slim(python_version="3.12")
    .uv_sync(uv_project_dir=".", frozen=True, extra_options="--no-dev")
    .add_local_python_source("socialguard_models")
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
GPU_STARTUP_TIMEOUT_SECONDS = 600
WORKER_TIMEOUT_SECONDS = 840
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
async def process_transcription(  # noqa: C901, PLR0915 - terminal-event orchestration has explicit failure logs.
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
    try:
        await _ModalLedger().set_state(ledger_key, "dispatched")
    except Exception as exc:
        log_event(
            "worker_dispatch_state_write_failed",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise
    terminal_key = f"terminal:{transcription_id}"
    try:
        existing = await JOB_LEDGER.get.aio(terminal_key)
    except Exception as exc:
        log_event(
            "terminal_event_lookup_failed",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise
    if isinstance(existing, bytes):
        log_event(
            "terminal_replay",
            transcription_id=transcription_id,
            report_id=command.report_id,
        )
        try:
            credentials = await asyncio.to_thread(_assume_aws_credentials)
        except Exception as exc:
            log_event(
                "worker_credentials_failed",
                level=logging.ERROR,
                transcription_id=transcription_id,
                report_id=command.report_id,
                error_type=type(exc).__name__,
                detail=str(exc),
                exc_info=True,
            )
            raise
        await _deliver_callback(
            command.callback_url, existing, credentials, command, transcription_id
        )
        return
    try:
        credentials = await asyncio.to_thread(_assume_aws_credentials)
    except Exception as exc:
        log_event(
            "worker_credentials_failed",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise
    try:
        log_event(
            "audio_download_started",
            transcription_id=transcription_id,
            report_id=command.report_id,
            audio_uri=command.audio_uri,
        )
        audio = await download_audio(command.audio_uri, credentials)
        log_event(
            "audio_downloaded",
            transcription_id=transcription_id,
            report_id=command.report_id,
            bytes_downloaded=len(audio),
        )
        log_event(
            "transcription_inference_started",
            transcription_id=transcription_id,
            report_id=command.report_id,
            model=command.model.value,
        )
        text = cast(
            "str",
            await GraniteModel().transcribe_bytes.remote.aio(  # pyright: ignore[reportUnknownMemberType,reportCallIssue]
                audio, transcription_id
            ),
        )
        if text:
            log_event(
                "transcription_inference_completed",
                transcription_id=transcription_id,
                report_id=command.report_id,
                transcript_characters=len(text),
            )
            event: dict[str, object] = {
                "type": "transcription.completed",
                "id": transcription_id,
                "report_id": command.report_id,
                "model": command.model.value,
                "data": {"text": text},
            }
        else:
            log_event(
                "transcription_empty_result",
                level=logging.WARNING,
                transcription_id=transcription_id,
                report_id=command.report_id,
            )
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
            report_id=command.report_id,
            model=command.model.value,
            audio_uri=command.audio_uri,
            reason=str(exc),
        )
        event = _failure_event(transcription_id, command, "audio_rejected", str(exc))
    except AudioRetrievalError as exc:
        log_event(
            "audio_retrieval_failed",
            level=logging.WARNING,
            transcription_id=transcription_id,
            report_id=command.report_id,
            audio_uri=command.audio_uri,
            error_type=type(exc).__name__,
            detail=str(exc),
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
            report_id=command.report_id,
            model=command.model.value,
            audio_uri=command.audio_uri,
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
    log_event(
        "terminal_event_persisting",
        transcription_id=transcription_id,
        report_id=command.report_id,
        event_type=event["type"],
        body_bytes=len(body),
    )
    try:
        await JOB_LEDGER.put.aio(terminal_key, body, skip_if_exists=True)
        winning = await JOB_LEDGER.get.aio(terminal_key)
    except Exception as exc:
        log_event(
            "terminal_event_persistence_failed",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise
    if not isinstance(winning, bytes):
        message = "terminal event persistence failed"
        log_event(
            "terminal_event_persistence_invalid",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            value_type=type(winning).__name__,
        )
        raise TypeError(message)
    log_event(
        "terminal_event_persisted",
        transcription_id=transcription_id,
        report_id=command.report_id,
        event_type=event["type"],
    )
    await _deliver_callback(
        command.callback_url, winning, credentials, command, transcription_id
    )
    log_event(
        "worker_job_finished",
        transcription_id=transcription_id,
        report_id=command.report_id,
        outcome=event["type"],
    )
    await asyncio.sleep(0)


async def _deliver_callback(
    url: str,
    body: bytes,
    credentials: TemporaryAwsCredentials,
    command: SubmissionCommand,
    transcription_id: str,
) -> None:
    """Send a bounded callback without logging request data or secrets."""
    import httpx

    try:
        validate_url(url, NetworkPolicy())
    except Exception as exc:
        log_event(
            "callback_url_rejected",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise

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
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            detail="Callback URL does not match the configured AWS target.",
        )
        return
    try:
        log_event(
            "callback_delivery_started",
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            body_bytes=len(body),
        )
        signer = _callback_signer(credentials)
        await deliver_with_retries(Sender(), url, body, signer, sleep=_sleep)
    except CallbackContractError as exc:
        log_event(
            "callback_contract_rejected",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            error_type=type(exc).__name__,
            detail="Callback payload was rejected and will not be retried locally.",
            exc_info=True,
        )
        return
    except CallbackAuthenticationError as exc:
        log_event(
            "callback_authentication_rejected",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            error_type=type(exc).__name__,
            detail="OIDC, IAM, or SigV4 callback authentication failed.",
            exc_info=True,
        )
        return
    except CallbackDeliveryError as exc:
        log_event(
            "callback_exhausted",
            level=logging.ERROR,
            transcription_id=transcription_id,
            report_id=command.report_id,
            callback_url=redacted_url(url),
            error_type=type(exc).__name__,
            detail=str(exc),
            exc_info=True,
        )
        raise
    log_event(
        "callback_delivery_completed",
        transcription_id=transcription_id,
        report_id=command.report_id,
        callback_url=redacted_url(url),
    )


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
        import torch  # pyright: ignore[reportMissingModuleSource]
        from huggingface_hub import (  # pyright: ignore[reportMissingModuleSource]
            snapshot_download,
        )
        from transformers import (  # pyright: ignore[reportMissingModuleSource]
            AutoModelForSpeechSeq2Seq,
            AutoProcessor,
        )

        log_event(
            "granite_model_loading",
            model_id=MODEL_ID,
            model_revision=MODEL_REVISION,
        )
        snapshot = Path(
            snapshot_download(
                repo_id=MODEL_ID,
                revision=MODEL_REVISION,
                cache_dir=CACHE_DIRECTORY,
                local_files_only=True,
            )
        )
        log_event("granite_model_snapshot_ready", snapshot=str(snapshot))
        self.processor = AutoProcessor.from_pretrained(snapshot, local_files_only=True)
        self.model = (
            AutoModelForSpeechSeq2Seq.from_pretrained(
                snapshot, local_files_only=True, torch_dtype=torch.bfloat16
            )
            .eval()
            .to("cuda")
        )
        log_event(
            "granite_model_loaded",
            model_id=MODEL_ID,
            model_revision=MODEL_REVISION,
        )

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def transcribe_bytes(self, audio_bytes: bytes, transcription_id: str) -> str:
        """Normalize uploaded audio then generate deterministic Granite output."""
        import torch  # pyright: ignore[reportMissingModuleSource]
        import torchaudio  # pyright: ignore[reportMissingModuleSource]

        log_event(
            "granite_inference_started",
            transcription_id=transcription_id,
            audio_bytes=len(audio_bytes),
        )
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
        text = self.processor.tokenizer.batch_decode(
            generated, add_special_tokens=False, skip_special_tokens=True
        )[0]
        log_event(
            "granite_inference_completed",
            transcription_id=transcription_id,
            transcript_characters=len(text),
        )
        return text


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
