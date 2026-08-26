"""Public request, response, and callback schemas for the ASR gateway."""

from enum import StrEnum
from typing import Annotated, Literal
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, Field, field_validator


class TranscriptionModel(StrEnum):
    """Models available through the gateway's stable public contract."""

    GRANITE_4_0_1B_SPEECH = "granite-4.0-1b-speech"


class CallbackErrorCode(StrEnum):
    """Terminal transcription failures safe to disclose to callback receivers."""

    AUDIO_REJECTED = "audio_rejected"
    AUDIO_RETRIEVAL_FAILED = "audio_retrieval_failed"
    INVALID_AUDIO = "invalid_audio"
    MODEL_UNAVAILABLE = "model_unavailable"
    TRANSCRIPTION_FAILED = "transcription_failed"
    INTERNAL_ERROR = "internal_error"


class GatewayErrorCode(StrEnum):
    """Errors returned directly by the gateway HTTP boundary."""

    AUTHENTICATION_REQUIRED = "authentication_required"
    FORBIDDEN = "forbidden"
    IDEMPOTENCY_CONFLICT = "idempotency_conflict"
    RATE_LIMITED = "rate_limited"
    INTERNAL_ERROR = "internal_error"
    MODEL_UNAVAILABLE = "model_unavailable"


class GatewayModel(BaseModel):
    """Base settings for immutable, strict gateway messages."""

    model_config = ConfigDict(extra="forbid", frozen=True)


class TranscriptionRequest(GatewayModel):
    """Request that submits one already-uploaded audio object for transcription."""

    model: TranscriptionModel = Field(
        description="Internal identifier of the model that will process the audio.",
        examples=[TranscriptionModel.GRANITE_4_0_1B_SPEECH],
    )
    audio_uri: Annotated[str, Field(max_length=8_192)] = Field(
        description=(
            "S3 URI for an object accessible to the worker's AWS identity. The worker "
            "uses its own credentials and enforces a download size limit."
        ),
        examples=["s3://audio-bucket/path/audio.wav"],
    )
    callback_url: Annotated[str, Field(max_length=8_192)] = Field(
        description=(
            "HTTPS endpoint for at-least-once completion or failure delivery. "
            "Receivers must deduplicate events by transcription ID."
        ),
        examples=["https://client.example.com/webhooks/transcriptions"],
    )
    report_id: Annotated[
        str,
        Field(min_length=1, max_length=255, pattern=r"^[1-9][0-9]*$"),
    ] = Field(
        description=(
            "Positive integer report identifier. It is retained with the job and "
            "echoed in the accepted response and every callback event for correlation."
        ),
        examples=["42"],
    )
    idempotency_key: Annotated[str, Field(min_length=1, max_length=255)] = Field(
        description=(
            "Client-generated key for safe retries. The eventual submission store must "
            "scope it to the authenticated caller and return the original job for an "
            "identical retry."
        ),
        examples=["customer-request-123"],
    )

    @field_validator("audio_uri")
    @classmethod
    def require_s3_uri(cls, value: str) -> str:
        """Require an S3 bucket and non-empty object key without URL parameters."""
        parsed = urlsplit(value)
        if (
            parsed.scheme != "s3"
            or not parsed.netloc
            or not parsed.path.lstrip("/")
            or parsed.username is not None
            or parsed.password is not None
            or parsed.port is not None
            or parsed.query
            or parsed.fragment
        ):
            message = "Audio location must be an S3 URI with a bucket and object key."
            raise ValueError(message)
        return value

    @field_validator("callback_url")
    @classmethod
    def require_https_url(cls, value: str) -> str:
        """Require an absolute HTTPS URL without applying retrieval policy."""
        parsed = urlsplit(value)
        if (
            parsed.scheme != "https"
            or not parsed.netloc
            or parsed.username is not None
            or parsed.password is not None
            or parsed.fragment
            or parsed.port not in (None, 443)
        ):
            message = (
                "URL must be an absolute HTTPS URL on port 443 without credentials "
                "or fragments."
            )
            raise ValueError(message)
        return value


class QueuedTranscription(GatewayModel):
    """Immediate response for a transcription accepted for asynchronous work."""

    id: str = Field(
        description="Stable identifier for the accepted transcription.",
        examples=["transcription_01J0EXAMPLE"],
    )
    report_id: str = Field(
        description="Opaque caller report identifier supplied in the submission.",
        examples=["report_01J0EXAMPLE"],
    )
    status: Literal["queued"] = Field(
        description="The gateway accepted the request and queued it for processing.",
    )


class ModelDescriptor(GatewayModel):
    """Public identity of a model served by this gateway."""

    id: TranscriptionModel = Field(
        description="Internal model identifier accepted by transcription requests.",
    )


class ModelList(GatewayModel):
    """Response listing public model identifiers."""

    data: tuple[ModelDescriptor, ...] = Field(
        description="Models currently accepted by this gateway.",
    )


class HealthResponse(GatewayModel):
    """Liveness response for the gateway process."""

    status: Literal["ok"] = Field(description="The gateway process is healthy.")


class CallbackData(GatewayModel):
    """Completed transcription payload delivered to a client callback URL."""

    text: str = Field(min_length=1, description="Final transcription text.")


class CallbackError(GatewayModel):
    """Stable error payload delivered when transcription fails."""

    code: CallbackErrorCode = Field(description="Machine-readable failure code.")
    message: str = Field(
        min_length=1, description="Safe human-readable failure description."
    )


class GatewayError(GatewayModel):
    """Stable machine- and human-readable HTTP error detail."""

    code: GatewayErrorCode = Field(description="Machine-readable failure code.")
    message: str = Field(description="Safe human-readable failure description.")
    param: str | None = Field(
        default=None,
        description="Request field responsible for the error, when applicable.",
    )
    request_id: str | None = Field(
        default=None,
        description="Gateway request identifier, when available.",
    )


class ErrorEnvelope(GatewayModel):
    """The error response body for public gateway operations."""

    error: GatewayError


class TranscriptionCompleted(GatewayModel):
    """At-least-once event sent after successful transcription."""

    type: Literal["transcription.completed"] = Field(
        description="Event type discriminator.",
    )
    id: str = Field(
        min_length=1, description="Transcription ID from the queued response."
    )
    report_id: str = Field(
        min_length=1,
        pattern=r"^[1-9][0-9]*$",
        description="Positive integer report identifier supplied in the submission.",
    )
    model: TranscriptionModel = Field(description="Model that processed the audio.")
    data: CallbackData


class TranscriptionFailed(GatewayModel):
    """At-least-once event sent when transcription cannot complete."""

    type: Literal["transcription.failed"] = Field(
        description="Event type discriminator.",
    )
    id: str = Field(
        min_length=1, description="Transcription ID from the queued response."
    )
    report_id: str = Field(
        min_length=1,
        pattern=r"^[1-9][0-9]*$",
        description="Positive integer report identifier supplied in the submission.",
    )
    model: TranscriptionModel = Field(description="Model selected by the request.")
    error: CallbackError


CallbackEvent = Annotated[
    TranscriptionCompleted | TranscriptionFailed,
    Field(discriminator="type"),
]
