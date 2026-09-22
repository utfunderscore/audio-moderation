import io
import json
from unittest.mock import Mock

import pytest
from botocore.exceptions import EndpointConnectionError

import socialguard_models.callbacks as callbacks
from socialguard_models.callbacks import get_callback_function_name, post_callback
from socialguard_models.transcription.callbacks import completion_outcome
from socialguard_models.transcription.contracts import CompletedOutcome, FailedOutcome


@pytest.fixture
def session(monkeypatch: pytest.MonkeyPatch) -> Mock:
    monkeypatch.setenv("AWS_REGION", "eu-west-2")
    return Mock()


def lambda_response(status_code: int) -> dict[str, object]:
    return {"StatusCode": 200, "Payload": io.BytesIO(json.dumps({"statusCode": status_code}).encode())}


@pytest.mark.parametrize(
    ("outcome", "expected_outcome"),
    [
        (
            CompletedOutcome(text="A complete transcript."),
            {
                "type": "success",
                "transcriptionResult": {
                    "jobId": "task-123",
                    "asrTaskId": "transcription-456",
                    "transcription": "A complete transcript.",
                },
            },
        ),
        (
            FailedOutcome(cause="EndpointConnectionError"),
            {
                "type": "failure",
                "error": "TranscriptionFailed",
                "cause": "EndpointConnectionError",
            },
        ),
    ],
)
def test_completion_callback_invokes_lambda_with_api_gateway_event(
    monkeypatch: pytest.MonkeyPatch,
    session: Mock,
    outcome: CompletedOutcome | FailedOutcome,
    expected_outcome: dict[str, object],
) -> None:
    lambda_client = session.client.return_value
    lambda_client.invoke.return_value = lambda_response(204)
    monkeypatch.setenv("TASK_CALLBACK_FUNCTION_NAME", "task-callback")

    post_callback(
        session=session,
        callback_function_name=get_callback_function_name(),
        task_token="token-123",
        outcome=completion_outcome(
            job_id="task-123", asr_task_id="transcription-456", outcome=outcome
        ),
    )

    session.client.assert_called_once()
    assert session.client.call_args.args == ("lambda",)
    assert session.client.call_args.kwargs["region_name"] == "eu-west-2"
    config = session.client.call_args.kwargs["config"]
    assert config.connect_timeout == callbacks.CALLBACK_REQUEST_TIMEOUT_SECONDS
    assert config.read_timeout == callbacks.CALLBACK_REQUEST_TIMEOUT_SECONDS
    invocation = lambda_client.invoke.call_args
    assert invocation.kwargs["FunctionName"] == "task-callback"
    assert invocation.kwargs["InvocationType"] == "RequestResponse"
    event = json.loads(invocation.kwargs["Payload"])
    assert event["version"] == "2.0"
    assert event["routeKey"] == "POST /callbacks/external-task"
    assert event["requestContext"]["http"]["method"] == "POST"
    assert json.loads(event["body"]) == {
        "taskToken": "token-123",
        "outcome": expected_outcome,
    }


def test_completion_callback_retries_transient_lambda_failures_with_backoff(
    monkeypatch: pytest.MonkeyPatch,
    session: Mock,
) -> None:
    lambda_client = session.client.return_value
    lambda_client.invoke.side_effect = [
        EndpointConnectionError(endpoint_url="https://lambda.eu-west-2.amazonaws.com"),
        lambda_response(503),
        lambda_response(204),
    ]
    wait = Mock()
    monkeypatch.setattr(callbacks, "sleep", wait)

    post_callback(
        session=session,
        callback_function_name="task-callback",
        task_token="token-123",
        outcome={"type": "success", "result": "text"},
    )

    assert lambda_client.invoke.call_count == 3
    assert [call.args[0] for call in wait.call_args_list] == [1, 2]


def test_completion_callback_does_not_retry_conflict(
    monkeypatch: pytest.MonkeyPatch,
    session: Mock,
) -> None:
    lambda_client = session.client.return_value
    lambda_client.invoke.return_value = lambda_response(409)
    wait = Mock()
    monkeypatch.setattr(callbacks, "sleep", wait)

    with pytest.raises(callbacks.CallbackDeliveryError) as raised:
        post_callback(
            session=session,
            callback_function_name="task-callback",
            task_token="token-123",
            outcome={"type": "success", "result": "text"},
        )

    assert raised.value.status_code == 409
    lambda_client.invoke.assert_called_once()
    wait.assert_not_called()


def test_completion_callback_raises_after_retries_are_exhausted(
    monkeypatch: pytest.MonkeyPatch,
    session: Mock,
) -> None:
    lambda_client = session.client.return_value
    lambda_client.invoke.side_effect = [
        lambda_response(503),
        lambda_response(503),
        lambda_response(503),
    ]
    wait = Mock()
    monkeypatch.setattr(callbacks, "sleep", wait)

    with pytest.raises(callbacks.CallbackDeliveryError) as raised:
        post_callback(
            session=session,
            callback_function_name="task-callback",
            task_token="token-123",
            outcome={"type": "failure", "cause": "RuntimeError"},
        )

    assert raised.value.status_code == 503
    assert lambda_client.invoke.call_count == callbacks.CALLBACK_MAX_ATTEMPTS
    assert [call.args[0] for call in wait.call_args_list] == [1, 2]


def test_completion_callback_requires_function_name(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("TASK_CALLBACK_FUNCTION_NAME", raising=False)

    with pytest.raises(RuntimeError, match="TASK_CALLBACK_FUNCTION_NAME"):
        get_callback_function_name()
