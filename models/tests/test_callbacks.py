import json
from email.message import Message
from unittest.mock import Mock
from urllib.error import HTTPError, URLError
from urllib.request import Request

import pytest

import socialguard_models.callbacks as callbacks
from socialguard_models.callbacks import (
    get_callback_uri,
    post_completion_callback,
)
from socialguard_models.transcription_contracts import CompletedOutcome, FailedOutcome


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
            FailedOutcome(cause="HTTPError"),
            {
                "type": "failure",
                "error": "TranscriptionFailed",
                "cause": "HTTPError",
            },
        ),
    ],
)
def test_completion_callback_posts_terminal_outcome(
    monkeypatch: pytest.MonkeyPatch,
    outcome: CompletedOutcome | FailedOutcome,
    expected_outcome: dict[str, object],
) -> None:
    response = Mock()
    response.__enter__ = Mock(return_value=response)
    response.__exit__ = Mock(return_value=None)
    callback = Mock(return_value=response)
    monkeypatch.setenv("TRANSCRIPTION_CALLBACK_URI", " https://example.com/callback ")
    monkeypatch.setattr(callbacks, "urlopen", callback)

    post_completion_callback(
        callback_uri=get_callback_uri(),
        task_token="token-123",
        job_id="task-123",
        asr_task_id="transcription-456",
        outcome=outcome,
    )

    request = callback.call_args.args[0]
    assert isinstance(request, Request)
    assert request.full_url == "https://example.com/callback"
    assert request.method == "POST"
    assert request.headers["Content-type"] == "application/json"
    assert isinstance(request.data, bytes)
    assert json.loads(request.data) == {
        "taskToken": "token-123",
        "outcome": expected_outcome,
    }
    callback.assert_called_once_with(
        request,
        timeout=callbacks.CALLBACK_REQUEST_TIMEOUT_SECONDS,
    )
    response.read.assert_called_once_with()


def test_completion_callback_retries_transient_failures_with_backoff(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    response = Mock()
    response.__enter__ = Mock(return_value=response)
    response.__exit__ = Mock(return_value=None)
    callback = Mock(
        side_effect=[
            URLError("unavailable"),
            HTTPError("", 502, "", Message(), None),
            response,
        ]
    )
    wait = Mock()
    monkeypatch.setenv("TRANSCRIPTION_CALLBACK_URI", "https://example.com/callback")
    monkeypatch.setattr(callbacks, "urlopen", callback)
    monkeypatch.setattr(callbacks, "sleep", wait)

    post_completion_callback(
        callback_uri=get_callback_uri(),
        task_token="token-123",
        job_id="task-123",
        asr_task_id="transcription-456",
        outcome=CompletedOutcome(text="text"),
    )

    assert callback.call_count == 3
    assert [call.args[0] for call in wait.call_args_list] == [1, 2]


def test_completion_callback_does_not_retry_permanent_http_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    error = HTTPError("", 400, "", Message(), None)
    callback = Mock(side_effect=error)
    wait = Mock()
    monkeypatch.setenv("TRANSCRIPTION_CALLBACK_URI", "https://example.com/callback")
    monkeypatch.setattr(callbacks, "urlopen", callback)
    monkeypatch.setattr(callbacks, "sleep", wait)

    with pytest.raises(HTTPError) as raised:
        post_completion_callback(
            callback_uri=get_callback_uri(),
            task_token="token-123",
            job_id="task-123",
            asr_task_id="transcription-456",
            outcome=CompletedOutcome(text="text"),
        )

    assert raised.value is error
    callback.assert_called_once()
    wait.assert_not_called()


def test_completion_callback_raises_after_retries_are_exhausted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    callback = Mock(side_effect=URLError("unavailable"))
    wait = Mock()
    monkeypatch.setenv("TRANSCRIPTION_CALLBACK_URI", "https://example.com/callback")
    monkeypatch.setattr(callbacks, "urlopen", callback)
    monkeypatch.setattr(callbacks, "sleep", wait)

    with pytest.raises(URLError):
        post_completion_callback(
            callback_uri=get_callback_uri(),
            task_token="token-123",
            job_id="task-123",
            asr_task_id="transcription-456",
            outcome=FailedOutcome(cause="RuntimeError"),
        )

    assert callback.call_count == callbacks.CALLBACK_MAX_ATTEMPTS
    assert [call.args[0] for call in wait.call_args_list] == [1, 2]


def test_completion_callback_requires_uri(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("TRANSCRIPTION_CALLBACK_URI", raising=False)

    with pytest.raises(RuntimeError, match="TRANSCRIPTION_CALLBACK_URI"):
        get_callback_uri()
