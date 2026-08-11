"""FastAPI application factory for the model-neutral ASR gateway."""

from typing import Annotated

from fastapi import APIRouter, Depends, FastAPI, Header, Request, status
from fastapi.responses import JSONResponse
from fastapi.security import HTTPBearer

from socialguard_models.api.auth import (
    CallerDependency,
    GatewayAuthenticationError,
    GatewaySettings,
    require_caller,
)
from socialguard_models.api.schemas import (
    CallbackEvent,
    ErrorEnvelope,
    GatewayError,
    GatewayErrorCode,
    HealthResponse,
    ModelDescriptor,
    ModelList,
    QueuedTranscription,
    TranscriptionModel,
    TranscriptionRequest,
)
from socialguard_models.api.submission import (
    IdempotencyConflictError,
    SubmissionCommand,
    SubmissionUnavailableError,
    TranscriptionSubmitter,
)

_CALLBACKS = APIRouter()
_BEARER_SECURITY = HTTPBearer(auto_error=False)

ErrorResponse = dict[int | str, dict[str, object]]

_ERROR_RESPONSES: ErrorResponse = {
    status.HTTP_401_UNAUTHORIZED: {
        "model": ErrorEnvelope,
        "description": "Missing or invalid bearer token.",
    },
    status.HTTP_403_FORBIDDEN: {
        "model": ErrorEnvelope,
        "description": (
            "Authenticated caller is not permitted to perform the operation."
        ),
    },
    status.HTTP_409_CONFLICT: {
        "model": ErrorEnvelope,
        "description": (
            "An idempotency key was reused with a different request payload."
        ),
    },
    status.HTTP_429_TOO_MANY_REQUESTS: {
        "model": ErrorEnvelope,
        "description": "The gateway cannot accept more work at this time.",
    },
    status.HTTP_500_INTERNAL_SERVER_ERROR: {
        "model": ErrorEnvelope,
        "description": (
            "Unexpected gateway failure; implementation details are omitted."
        ),
    },
    status.HTTP_503_SERVICE_UNAVAILABLE: {
        "model": ErrorEnvelope,
        "description": "The selected model or submission dependency is unavailable.",
    },
}


@_CALLBACKS.post(
    "{$request.body#/callback_url}",
    response_model=None,
    summary="Transcription callback",
    description=(
        "The gateway delivers exactly one terminal event type at least once and "
        "retries until a configured retry window expires. Acknowledgement is any 2xx "
        "response. "
        "Receivers must deduplicate by transcription ID. The gateway signs the exact "
        "unmodified JSON request-body bytes using HMAC-SHA256."
    ),
)
async def transcription_callback(
    event: CallbackEvent,
    x_socialguard_timestamp: Annotated[
        str,
        Header(
            description=(
                "Unix timestamp in seconds authenticated by the callback signature."
            ),
        ),
    ],
    x_socialguard_webhook_key_id: Annotated[
        str,
        Header(
            description="Identifier of the HMAC signing key used for this delivery.",
        ),
    ],
    x_socialguard_signature: Annotated[
        str,
        Header(
            description=(
                "HMAC-SHA256 signature formatted as `sha256=<lowercase-hex>` over "
                "ASCII(timestamp) + `.` + the exact JSON request-body bytes."
            ),
        ),
    ],
) -> None:
    """Document the outbound callback without registering a local route."""
    del event, x_socialguard_timestamp, x_socialguard_webhook_key_id
    del x_socialguard_signature


def _error_response(
    status_code: int,
    code: GatewayErrorCode,
    message: str,
    *,
    param: str | None = None,
) -> JSONResponse:
    """Create a public error response with the stable envelope."""
    body = ErrorEnvelope(error=GatewayError(code=code, message=message, param=param))
    return JSONResponse(status_code=status_code, content=body.model_dump(mode="json"))


def create_gateway_app(
    submitter: TranscriptionSubmitter,
    settings: GatewaySettings,
) -> FastAPI:
    """Create a gateway application bound to one asynchronous submitter."""
    app = FastAPI(
        title="SocialGuard ASR Gateway",
        version="0.1.0",
        summary="Submit presigned audio URLs for asynchronous transcription.",
    )

    @app.exception_handler(GatewayAuthenticationError)
    async def handle_authentication_error(  # pyright: ignore[reportUnusedFunction]
        request: Request,
        exc: GatewayAuthenticationError,
    ) -> JSONResponse:
        """Map authentication failures to the public error envelope."""
        del request, exc
        return _error_response(
            status.HTTP_401_UNAUTHORIZED,
            GatewayErrorCode.AUTHENTICATION_REQUIRED,
            "A valid bearer token is required.",
        )

    @app.exception_handler(Exception)
    async def handle_unexpected_error(  # pyright: ignore[reportUnusedFunction]
        request: Request,
        exc: Exception,
    ) -> JSONResponse:
        """Do not expose implementation failures through the public API."""
        del request, exc
        return _error_response(
            status.HTTP_500_INTERNAL_SERVER_ERROR,
            GatewayErrorCode.INTERNAL_ERROR,
            "The gateway encountered an unexpected error.",
        )

    caller_dependency: CallerDependency = require_caller(settings)

    @app.post(
        "/v1/transcriptions",
        status_code=status.HTTP_202_ACCEPTED,
        callbacks=_CALLBACKS.routes,
        response_model=QueuedTranscription,
        summary="Queue a transcription",
        operation_id="queueTranscription",
        responses=_ERROR_RESPONSES,
    )
    async def queue_transcription(  # pyright: ignore[reportUnusedFunction]
        request: TranscriptionRequest,
        caller_id: Annotated[str, Depends(caller_dependency)],
    ) -> QueuedTranscription | JSONResponse:
        """Map an accepted HTTP request to the asynchronous submission seam."""
        try:
            result = await submitter.submit(
                SubmissionCommand(
                    caller_id=caller_id,
                    model=request.model,
                    audio_url=request.audio_url,
                    callback_url=request.callback_url,
                    report_id=request.report_id,
                    idempotency_key=request.idempotency_key,
                ),
            )
        except IdempotencyConflictError:
            return _error_response(
                status.HTTP_409_CONFLICT,
                GatewayErrorCode.IDEMPOTENCY_CONFLICT,
                "The idempotency key was already used with a different request.",
                param="idempotency_key",
            )
        except SubmissionUnavailableError:
            return _error_response(
                status.HTTP_503_SERVICE_UNAVAILABLE,
                GatewayErrorCode.MODEL_UNAVAILABLE,
                "The gateway could not queue the transcription.",
            )
        return QueuedTranscription(
            id=result.transcription_id,
            report_id=request.report_id,
            status="queued",
        )

    @app.get(
        "/v1/models",
        summary="List accepted models",
        operation_id="listModels",
        responses=_ERROR_RESPONSES,
    )
    async def list_models(  # pyright: ignore[reportUnusedFunction]
        caller_id: Annotated[str, Depends(caller_dependency)],
    ) -> ModelList:
        """List public internal model identifiers for the authenticated caller."""
        del caller_id
        return ModelList(
            data=(ModelDescriptor(id=TranscriptionModel.GRANITE_4_0_1B_SPEECH),),
        )

    @app.get("/healthz", summary="Check gateway health", operation_id="healthCheck")
    async def health() -> HealthResponse:  # pyright: ignore[reportUnusedFunction]
        """Return process liveness without checking downstream dependencies."""
        return HealthResponse(status="ok")

    return app
