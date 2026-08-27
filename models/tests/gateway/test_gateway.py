"""Tests for the local ASR gateway HTTP and OpenAPI contract."""

import json
import logging
from collections.abc import Mapping
from typing import Protocol, cast

import pytest
from fastapi.testclient import TestClient
from httpx import Response
from starlette import status

from socialguard_models.api.app import create_gateway_app
from socialguard_models.api.auth import GatewaySettings
from socialguard_models.api.generate_openapi import OPENAPI_ARTIFACT, render_openapi
from socialguard_models.api.schemas import TranscriptionModel
from socialguard_models.api.submission import (
    IdempotencyConflictError,
    SubmissionCommand,
    SubmissionResult,
    SubmissionUnavailableError,
)

EXPECTED_CALLBACK_EVENT_VARIANTS = 2
_TEST_API_TOKEN = "test-token"  # noqa: S105


class GatewayClient(Protocol):
    """Typed HTTP surface used by gateway contract tests."""

    def get(self, url: str) -> Response:
        """Send one GET request."""
        ...

    def post(self, url: str, *, json: object) -> Response:
        """Send one JSON POST request."""
        ...


class FakeSubmitter:
    """Typed in-memory submission seam for HTTP contract tests."""

    def __init__(self) -> None:
        """Initialize the fake without a received command."""
        self.command: SubmissionCommand | None = None

    async def submit(self, command: SubmissionCommand) -> SubmissionResult:
        """Record the command and return a deterministic job identifier."""
        self.command = command
        return SubmissionResult(transcription_id="transcription_01J0EXAMPLE")


def _client() -> tuple[GatewayClient, FakeSubmitter]:
    """Create an isolated client and submission fake."""
    submitter = FakeSubmitter()
    client = TestClient(
        create_gateway_app(submitter, GatewaySettings(api_token=_TEST_API_TOKEN)),
        headers={"Authorization": f"Bearer {_TEST_API_TOKEN}"},
    )
    return cast("GatewayClient", client), submitter


def _valid_request() -> dict[str, str]:
    """Return a valid asynchronous transcription request."""
    return {
        "model": "granite-4.0-1b-speech",
        "audio_uri": "s3://audio-bucket/path/audio.wav",
        "callback_url": "https://client.example.com/hooks/transcriptions",
        "report_id": "42",
        "idempotency_key": "customer-request-123",
    }


def test_queue_transcription_maps_request_to_submission_command() -> None:
    """The endpoint maps the accepted request exactly to the async seam."""
    client, submitter = _client()

    response = client.post("/v1/transcriptions", json=_valid_request())

    assert response.status_code == status.HTTP_202_ACCEPTED
    assert response.json() == {
        "id": "transcription_01J0EXAMPLE",
        "report_id": "42",
        "status": "queued",
    }
    assert submitter.command == SubmissionCommand(
        caller_id="single-tenant",
        model=TranscriptionModel.GRANITE_4_0_1B_SPEECH,
        audio_uri="s3://audio-bucket/path/audio.wav",
        callback_url="https://client.example.com/hooks/transcriptions",
        report_id="42",
        idempotency_key="customer-request-123",
    )


def test_gateway_requires_configured_bearer_token() -> None:
    """Protected API routes reject missing and invalid bearer tokens uniformly."""
    submitter = FakeSubmitter()
    app = create_gateway_app(submitter, GatewaySettings(api_token=_TEST_API_TOKEN))

    missing_response = cast(
        "Response",
        TestClient(app).post(  # pyright: ignore[reportUnknownMemberType]
            "/v1/transcriptions", json=_valid_request()
        ),
    )
    invalid_response = cast(
        "Response",
        TestClient(app).post(  # pyright: ignore[reportUnknownMemberType]
            "/v1/transcriptions",
            json=_valid_request(),
            headers={"Authorization": "Bearer wrong-token"},
        ),
    )
    health_response = cast(
        "Response",
        TestClient(app).get(  # pyright: ignore[reportUnknownMemberType]
            "/healthz"
        ),
    )

    expected_body = {
        "error": {
            "code": "authentication_required",
            "message": "A valid bearer token is required.",
            "param": None,
            "request_id": None,
        }
    }
    assert missing_response.status_code == status.HTTP_401_UNAUTHORIZED
    assert missing_response.json() == expected_body
    assert invalid_response.status_code == status.HTTP_401_UNAUTHORIZED
    assert invalid_response.json() == expected_body
    assert health_response.status_code == status.HTTP_200_OK


def test_gateway_logs_authentication_rejections_with_request_outcome(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Authentication rejection logs include a safe reason and HTTP outcome."""
    submitter = FakeSubmitter()
    app = create_gateway_app(submitter, GatewaySettings(api_token=_TEST_API_TOKEN))

    with caplog.at_level(logging.INFO, logger="socialguard"):
        response = cast(
            "Response",
            TestClient(app).post(  # pyright: ignore[reportUnknownMemberType]
                "/v1/transcriptions", json=_valid_request()
            ),
        )

    events = [json.loads(record.message) for record in caplog.records]
    assert response.status_code == status.HTTP_401_UNAUTHORIZED
    assert {
        "event": "authentication_rejected",
        "method": "POST",
        "path": "/v1/transcriptions",
        "reason": "missing_bearer_token",
    } in events
    assert {
        "event": "api_request_rejected",
        "method": "POST",
        "path": "/v1/transcriptions",
        "status_code": 401,
    }.items() <= events[-1].items()


def test_queue_transcription_rejects_idempotency_conflict() -> None:
    """Submission idempotency conflicts use the stable HTTP error envelope."""

    class ConflictSubmitter:
        """Submitter that reports a conflicting request key."""

        async def submit(self, command: SubmissionCommand) -> SubmissionResult:
            """Reject the supplied command as a conflicting retry."""
            del command
            raise IdempotencyConflictError

    client = cast(
        "GatewayClient",
        TestClient(
            create_gateway_app(
                ConflictSubmitter(), GatewaySettings(api_token=_TEST_API_TOKEN)
            ),
            headers={"Authorization": f"Bearer {_TEST_API_TOKEN}"},
        ),
    )

    response = client.post("/v1/transcriptions", json=_valid_request())

    assert response.status_code == status.HTTP_409_CONFLICT
    assert response.json() == {
        "error": {
            "code": "idempotency_conflict",
            "message": "The idempotency key was already used with a different request.",
            "param": "idempotency_key",
            "request_id": None,
        }
    }


def test_queue_transcription_returns_503_when_dispatch_is_unavailable() -> None:
    """An unspawned submission never receives a false queued response."""

    class UnavailableSubmitter:
        async def submit(self, command: SubmissionCommand) -> SubmissionResult:
            del command
            raise SubmissionUnavailableError

    client = cast(
        "GatewayClient",
        TestClient(
            create_gateway_app(
                UnavailableSubmitter(), GatewaySettings(api_token=_TEST_API_TOKEN)
            ),
            headers={"Authorization": f"Bearer {_TEST_API_TOKEN}"},
        ),
    )

    response = client.post("/v1/transcriptions", json=_valid_request())

    assert response.status_code == status.HTTP_503_SERVICE_UNAVAILABLE
    assert response.json() == {
        "error": {
            "code": "model_unavailable",
            "message": "The gateway could not queue the transcription.",
            "param": None,
            "request_id": None,
        }
    }


def test_gateway_redacts_unexpected_submitter_failure() -> None:
    """Unexpected implementation failures use the documented JSON error envelope."""

    class BrokenSubmitter:
        async def submit(self, command: SubmissionCommand) -> SubmissionResult:
            del command
            message = "secret implementation detail"
            raise RuntimeError(message)

    response = cast(
        "Response",
        TestClient(  # pyright: ignore[reportUnknownMemberType]
            create_gateway_app(
                BrokenSubmitter(), GatewaySettings(api_token=_TEST_API_TOKEN)
            ),
            raise_server_exceptions=False,
            headers={"Authorization": f"Bearer {_TEST_API_TOKEN}"},
        ).post("/v1/transcriptions", json=_valid_request()),
    )

    assert response.status_code == status.HTTP_500_INTERNAL_SERVER_ERROR
    assert response.json() == {
        "error": {
            "code": "internal_error",
            "message": "The gateway encountered an unexpected error.",
            "param": None,
            "request_id": None,
        }
    }


def test_queue_transcription_rejects_unknown_internal_model() -> None:
    """Only the closed internal model enum is accepted."""
    client, _ = _client()
    request = _valid_request() | {"model": "ibm-granite/granite-4.0-1b-speech"}

    response = client.post("/v1/transcriptions", json=request)

    assert response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_queue_transcription_rejects_extra_fields() -> None:
    """Gateway request models reject unknown input rather than ignoring it."""
    client, _ = _client()
    request = _valid_request() | {"unexpected": "value"}

    response = client.post("/v1/transcriptions", json=request)

    assert response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_queue_transcription_requires_s3_audio_uri() -> None:
    """The gateway requires an S3 bucket and object key without URL parameters."""
    client, _ = _client()
    for audio_uri in (
        "https://audio-bucket.s3.eu-west-2.amazonaws.com/audio.wav",
        "s3://audio-bucket",
        "s3://audio-bucket/audio.wav?versionId=1",
        "s3://audio-bucket/audio.wav#fragment",
    ):
        response = client.post(
            "/v1/transcriptions",
            json=_valid_request() | {"audio_uri": audio_uri},
        )
        assert response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_queue_transcription_requires_https_callback_url() -> None:
    """The gateway rejects non-HTTPS callback locations."""
    client, _ = _client()
    request = _valid_request() | {"callback_url": "http://client.example.com/hook"}

    response = client.post("/v1/transcriptions", json=request)

    assert response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_queue_transcription_requires_positive_integer_report_id() -> None:
    """The callback report identifier is a bounded positive integer string."""
    client, _ = _client()

    missing_response = client.post(
        "/v1/transcriptions",
        json={
            key: value for key, value in _valid_request().items() if key != "report_id"
        },
    )
    empty_response = client.post(
        "/v1/transcriptions",
        json=_valid_request() | {"report_id": ""},
    )
    long_response = client.post(
        "/v1/transcriptions",
        json=_valid_request() | {"report_id": "a" * 256},
    )

    assert missing_response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT
    assert empty_response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT
    assert long_response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT
    for invalid_report_id in ("0", "-1", "report_42"):
        response = client.post(
            "/v1/transcriptions",
            json=_valid_request() | {"report_id": invalid_report_id},
        )
        assert response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_queue_transcription_enforces_idempotency_key_bounds() -> None:
    """Idempotency keys must contain one to 255 characters."""
    client, _ = _client()

    empty_response = client.post(
        "/v1/transcriptions",
        json=_valid_request() | {"idempotency_key": ""},
    )
    long_response = client.post(
        "/v1/transcriptions",
        json=_valid_request() | {"idempotency_key": "a" * 256},
    )

    assert empty_response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT
    assert long_response.status_code == status.HTTP_422_UNPROCESSABLE_CONTENT


def test_models_and_health_endpoints() -> None:
    """The gateway exposes its model enum and liveness state."""
    client, _ = _client()

    models_response = client.get("/v1/models")
    health_response = client.get("/healthz")

    assert models_response.status_code == status.HTTP_200_OK
    assert models_response.json() == {"data": [{"id": "granite-4.0-1b-speech"}]}
    assert health_response.status_code == status.HTTP_200_OK
    assert health_response.json() == {"status": "ok"}


def _mapping_value(document: Mapping[str, object], key: str) -> object:
    """Return a JSON object value without widening it to an unknown type."""
    return document[key]


def _as_mapping(value: object) -> Mapping[str, object]:
    """Assert that one parsed JSON value is an object."""
    assert isinstance(value, Mapping)
    return cast("Mapping[str, object]", value)


def test_openapi_documents_callback_expression_and_event_union() -> None:
    """The generated definition describes callback delivery and both event variants."""
    document = cast("Mapping[str, object]", json.loads(render_openapi()))
    paths = _as_mapping(_mapping_value(document, "paths"))
    operation = _as_mapping(_mapping_value(paths, "/v1/transcriptions"))
    post = _as_mapping(_mapping_value(operation, "post"))
    callbacks = _as_mapping(_mapping_value(post, "callbacks"))
    callback = _as_mapping(next(iter(callbacks.values())))
    expression = _as_mapping(_mapping_value(callback, "{$request.body#/callback_url}"))
    callback_post = _as_mapping(_mapping_value(expression, "post"))
    request_body = _as_mapping(_mapping_value(callback_post, "requestBody"))
    content = _as_mapping(_mapping_value(request_body, "content"))
    application_json = _as_mapping(_mapping_value(content, "application/json"))
    schema = _as_mapping(_mapping_value(application_json, "schema"))
    discriminator = _as_mapping(_mapping_value(schema, "discriminator"))
    assert _mapping_value(discriminator, "propertyName") == "type"
    one_of = _mapping_value(schema, "oneOf")
    assert isinstance(one_of, list)
    typed_one_of = cast("list[object]", one_of)
    assert len(typed_one_of) == EXPECTED_CALLBACK_EVENT_VARIANTS
    assert "parameters" not in callback_post
    responses = _as_mapping(_mapping_value(callback_post, "responses"))
    assert "204" in responses


def test_openapi_documents_authentication_and_error_contract() -> None:
    """The generated definition exposes bearer auth and stable error responses."""
    document = cast("Mapping[str, object]", json.loads(render_openapi()))
    paths = _as_mapping(_mapping_value(document, "paths"))
    operation = _as_mapping(_mapping_value(paths, "/v1/transcriptions"))
    post = _as_mapping(_mapping_value(operation, "post"))
    responses = _as_mapping(_mapping_value(post, "responses"))
    components = _as_mapping(_mapping_value(document, "components"))
    security_schemes = _as_mapping(_mapping_value(components, "securitySchemes"))
    schemas = _as_mapping(_mapping_value(components, "schemas"))

    assert _mapping_value(post, "security") == [{"HTTPBearer": []}]
    assert {"401", "403", "409", "429", "500", "503"} <= responses.keys()
    assert _mapping_value(security_schemes, "HTTPBearer") == {
        "scheme": "bearer",
        "type": "http",
    }
    callback_error = _as_mapping(_mapping_value(schemas, "CallbackError"))
    callback_properties = _as_mapping(_mapping_value(callback_error, "properties"))
    callback_code = _as_mapping(_mapping_value(callback_properties, "code"))
    assert (
        _mapping_value(callback_code, "$ref")
        == "#/components/schemas/CallbackErrorCode"
    )


def test_checked_in_openapi_artifact_matches_generator() -> None:
    """The committed API definition is the deterministic app output."""
    assert OPENAPI_ARTIFACT.read_text(encoding="utf-8") == render_openapi()
