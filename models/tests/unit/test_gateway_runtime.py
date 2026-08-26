"""Tests for asynchronous gateway primitives that do not require Modal resources."""

import asyncio

import pytest
from pydantic import ValidationError

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
    UnsafeUrlError,
    validate_url,
)
from socialguard_models.api.schemas import TranscriptionModel
from socialguard_models.api.submission import (
    IdempotencyConflictError,
    LedgerSubmitter,
    SubmissionCommand,
    SubmissionRecord,
    SubmissionResult,
    SubmissionUnavailableError,
)


class MemoryLedger:
    """Atomic-in-one-loop ledger fake."""

    def __init__(self) -> None:
        self.records: dict[str, SubmissionRecord] = {}

    async def get(self, key: str) -> SubmissionRecord | None:
        return self.records.get(key)

    async def put_if_absent(self, key: str, record: SubmissionRecord) -> bool:
        if key in self.records:
            return False
        self.records[key] = record
        return True

    async def set_state(self, key: str, state: str) -> None:
        record = self.records[key]
        self.records[key] = SubmissionRecord(
            record.transcription_id, record.fingerprint, state, record.command
        )

    async def claim_dispatch(self, key: str) -> bool:
        claim_key = f"claim:{key}"
        if claim_key in self.records:
            return False
        self.records[claim_key] = self.records[key]
        return True

    async def release_dispatch_claim(self, key: str) -> None:
        self.records.pop(f"claim:{key}", None)


class Dispatcher:
    """Collect scheduled work without invoking Modal."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str]] = []

    async def dispatch(
        self,
        ledger_key: str,
        transcription_id: str,
        command: SubmissionCommand,
    ) -> None:
        del command
        self.calls.append((ledger_key, transcription_id))


def _command(*, report_id: str = "report") -> SubmissionCommand:
    return SubmissionCommand(
        caller_id="caller",
        model=TranscriptionModel.GRANITE_4_0_1B_SPEECH,
        audio_uri="s3://audio-bucket/file.wav",
        callback_url="https://hooks.example.com/callback",
        report_id=report_id,
        idempotency_key="retry-key",
    )


def test_ledger_submitter_replays_original_id_and_repairs_pending_work() -> None:
    """An identical retry returns the first ID and can repair a stranded dispatch."""
    ledger = MemoryLedger()
    dispatcher = Dispatcher()
    submitter = LedgerSubmitter(ledger, dispatcher)

    first = asyncio.run(submitter.submit(_command()))
    second = asyncio.run(submitter.submit(_command()))

    assert first == second
    assert len(dispatcher.calls) == 1
    assert dispatcher.calls[0][1] == first.transcription_id
    assert dispatcher.calls[0][0].startswith("submission:")


def test_ledger_submitter_rejects_different_payload_for_same_key() -> None:
    """Caller-scoped idempotency does not silently accept changed work."""
    submitter = LedgerSubmitter(MemoryLedger(), Dispatcher())
    asyncio.run(submitter.submit(_command()))

    with pytest.raises(IdempotencyConflictError):
        asyncio.run(submitter.submit(_command(report_id="different")))


def test_concurrent_idempotent_submissions_dispatch_once() -> None:
    """A separate atomic dispatch claim prevents duplicate spawned workers."""
    ledger = MemoryLedger()
    dispatcher = Dispatcher()
    submitter = LedgerSubmitter(ledger, dispatcher)

    async def submit_twice() -> tuple[SubmissionResult, SubmissionResult]:
        return await asyncio.gather(
            submitter.submit(_command()), submitter.submit(_command())
        )

    first, second = asyncio.run(submit_twice())

    assert first == second
    assert [call[1] for call in dispatcher.calls] == [first.transcription_id]


def test_state_write_after_spawn_returns_stable_accepted_job() -> None:
    """The non-transactional spawn/state gap cannot cause an ambiguous HTTP retry."""

    class StateFailingLedger(MemoryLedger):
        async def set_state(self, key: str, state: str) -> None:
            del key, state
            message = "state persistence failed"
            raise OSError(message)

    ledger = StateFailingLedger()

    class RepairingDispatcher(Dispatcher):
        async def dispatch(
            self,
            ledger_key: str,
            transcription_id: str,
            command: SubmissionCommand,
        ) -> None:
            await super().dispatch(ledger_key, transcription_id, command)
            record = ledger.records[ledger_key]
            ledger.records[ledger_key] = SubmissionRecord(
                record.transcription_id,
                record.fingerprint,
                "dispatched",
                record.command,
            )

    dispatcher = RepairingDispatcher()
    submitter = LedgerSubmitter(ledger, dispatcher)
    first = asyncio.run(submitter.submit(_command()))
    second = asyncio.run(submitter.submit(_command()))

    assert first == second
    assert [call[1] for call in dispatcher.calls] == [first.transcription_id]


def test_concurrent_replay_waits_for_owner_then_returns_dispatched_id() -> None:
    """A claim loser cannot return before the owner has successfully dispatched."""

    class BlockingDispatcher(Dispatcher):
        def __init__(self) -> None:
            super().__init__()
            self.owner_started = asyncio.Event()
            self.release_owner = asyncio.Event()

        async def dispatch(
            self,
            ledger_key: str,
            transcription_id: str,
            command: SubmissionCommand,
        ) -> None:
            await super().dispatch(ledger_key, transcription_id, command)
            self.owner_started.set()
            await self.release_owner.wait()

    async def run() -> tuple[SubmissionResult, SubmissionResult, Dispatcher]:
        ledger = MemoryLedger()
        dispatcher = BlockingDispatcher()
        waiter_sleep_started = asyncio.Event()

        async def wait_for_owner(seconds: float) -> None:
            del seconds
            waiter_sleep_started.set()
            await dispatcher.release_owner.wait()

        submitter = LedgerSubmitter(
            ledger,
            dispatcher,
            handoff_attempts=2,
            sleeper=wait_for_owner,
        )
        owner = asyncio.create_task(submitter.submit(_command()))
        await dispatcher.owner_started.wait()
        waiter = asyncio.create_task(submitter.submit(_command()))
        await waiter_sleep_started.wait()
        assert not waiter.done()
        dispatcher.release_owner.set()
        return await owner, await waiter, dispatcher

    first, second, dispatcher = asyncio.run(run())

    assert first == second
    assert [call[1] for call in dispatcher.calls] == [first.transcription_id]


def test_waiting_replay_dispatches_after_owner_spawn_failure() -> None:
    """A released failed claim lets a waiting identical retry dispatch the job."""

    class FailFirstDispatcher(Dispatcher):
        def __init__(self) -> None:
            super().__init__()
            self.owner_started = asyncio.Event()
            self.fail_owner = asyncio.Event()

        async def dispatch(
            self,
            ledger_key: str,
            transcription_id: str,
            command: SubmissionCommand,
        ) -> None:
            await super().dispatch(ledger_key, transcription_id, command)
            if len(self.calls) == 1:
                self.owner_started.set()
                await self.fail_owner.wait()
                message = "spawn failed"
                raise RuntimeError(message)

    async def run() -> tuple[SubmissionResult, Dispatcher]:
        ledger = MemoryLedger()
        dispatcher = FailFirstDispatcher()
        continue_waiter = asyncio.Event()

        async def wait_for_failure(seconds: float) -> None:
            del seconds
            await continue_waiter.wait()

        submitter = LedgerSubmitter(
            ledger,
            dispatcher,
            handoff_attempts=2,
            sleeper=wait_for_failure,
        )
        owner = asyncio.create_task(submitter.submit(_command()))
        await dispatcher.owner_started.wait()
        waiter = asyncio.create_task(submitter.submit(_command()))
        await asyncio.sleep(0)
        dispatcher.fail_owner.set()
        with pytest.raises(SubmissionUnavailableError):
            await owner
        continue_waiter.set()
        return await waiter, dispatcher

    result, dispatcher = asyncio.run(run())

    assert len(dispatcher.calls) == 2
    assert dispatcher.calls[0][1] == result.transcription_id
    assert dispatcher.calls[1][1] == result.transcription_id


def test_stuck_dispatch_claim_returns_unavailable_instead_of_queued() -> None:
    """A bounded handoff never confirms work when no dispatcher owns it."""

    class StuckClaimLedger(MemoryLedger):
        async def claim_dispatch(self, key: str) -> bool:
            del key
            return False

    async def no_wait(seconds: float) -> None:
        del seconds

    submitter = LedgerSubmitter(
        StuckClaimLedger(),
        Dispatcher(),
        handoff_attempts=2,
        sleeper=no_wait,
    )

    with pytest.raises(SubmissionUnavailableError):
        asyncio.run(submitter.submit(_command()))


def test_network_policy_rejects_private_addresses_and_url_ambiguity() -> None:
    """Network policy rejects private DNS results and unsafe URL components."""
    with pytest.raises(UnsafeUrlError):
        validate_url("https://example.com", resolver=lambda host, port: ["127.0.0.1"])
    with pytest.raises(UnsafeUrlError):
        validate_url(
            "https://user@example.com", resolver=lambda host, port: ["8.8.8.8"]
        )
    assert (
        validate_url(
            "https://audio.example.com/file",
            NetworkPolicy(frozenset({"audio.example.com"})),
            resolver=lambda host, port: ["8.8.8.8"],
        ).hostname
        == "audio.example.com"
    )


def _callback_signer() -> SigV4CallbackSigner:
    return SigV4CallbackSigner(
        access_key_id="temporary-access-key",
        secret_access_key="temporary-secret-key",  # noqa: S106
        session_token="temporary-session-token",  # noqa: S106
        region="eu-west-2",
        service="lambda",
    )


def test_callback_retry_signs_the_same_body_until_204_acknowledges() -> None:
    """Retries preserve exact bytes and include the temporary AWS session token."""

    class Sender:
        def __init__(self) -> None:
            self.sent: list[tuple[bytes, dict[str, str]]] = []

        async def send(self, url: str, body: bytes, headers: dict[str, str]) -> int:
            del url
            self.sent.append((body, headers))
            return 500 if len(self.sent) == 1 else 204

    sender = Sender()
    body = b'{"id":"transcription_1"}'
    delivery = asyncio.run(
        deliver_with_retries(
            sender,
            "https://example.lambda-url.eu-west-2.on.aws/",
            body,
            _callback_signer(),
        )
    )

    assert delivery.delivered and delivery.attempts == 2
    assert sender.sent[0][0] == sender.sent[1][0] == body
    assert sender.sent[0][1]["X-Amz-Security-Token"] == "temporary-session-token"
    assert "/eu-west-2/lambda/aws4_request" in sender.sent[0][1]["Authorization"]
    assert sender.sent[0][1]["Content-Type"] == "application/json"


@pytest.mark.parametrize(
    ("status_code", "error_type"),
    [
        (400, CallbackContractError),
        (403, CallbackAuthenticationError),
    ],
)
def test_callback_does_not_retry_permanent_rejections(
    status_code: int, error_type: type[CallbackDeliveryError]
) -> None:
    """Payload and authentication failures stop local callback retries."""

    class Sender:
        attempts = 0

        async def send(self, url: str, body: bytes, headers: dict[str, str]) -> int:
            del url, body, headers
            self.attempts += 1
            return status_code

    sender = Sender()
    with pytest.raises(error_type):
        asyncio.run(
            deliver_with_retries(
                sender,
                "https://example.lambda-url.eu-west-2.on.aws/",
                b"{}",
                _callback_signer(),
            )
        )
    assert sender.attempts == 1


def test_callback_exhaustion_raises_for_modal_retry() -> None:
    """A non-acknowledging endpoint leaves the persisted event retryable."""

    class Sender:
        async def send(self, url: str, body: bytes, headers: dict[str, str]) -> int:
            del url, body, headers
            return 500

    with pytest.raises(CallbackDeliveryError):
        asyncio.run(
            deliver_with_retries(
                Sender(),
                "https://example.lambda-url.eu-west-2.on.aws/",
                b"{}",
                _callback_signer(),
            )
        )


def test_callback_body_bytes_change_the_sigv4_signature() -> None:
    """SigV4 authenticates the exact callback body bytes sent over HTTP."""
    signer = _callback_signer()
    url = "https://example.lambda-url.eu-west-2.on.aws/"
    first = signer.headers(url, b"{}")
    second = signer.headers(url, b'{"x":1}')
    assert first["Authorization"] != second["Authorization"]


@pytest.mark.parametrize(
    "event",
    [
        {
            "type": "transcription.completed",
            "id": "transcription_1",
            "report_id": "0",
            "model": "granite-4.0-1b-speech",
            "data": {"text": "text"},
        },
        {
            "type": "transcription.completed",
            "id": "transcription_1",
            "report_id": "42",
            "model": "granite-4.0-1b-speech",
            "data": {"text": ""},
        },
        {
            "type": "transcription.failed",
            "id": "transcription_1",
            "report_id": "42",
            "model": "granite-4.0-1b-speech",
            "error": {
                "code": "transcription_failed",
                "message": "safe",
                "unknown": "rejected",
            },
        },
    ],
)
def test_terminal_body_rejects_payloads_the_callback_contract_forbids(
    event: dict[str, object],
) -> None:
    """Invalid report IDs, empty strings, and unknown fields never reach AWS."""
    with pytest.raises(ValidationError):
        terminal_body(event)
