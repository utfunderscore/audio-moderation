"""Submission idempotency and the seam to asynchronous transcription work."""

import asyncio
import json
import logging
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from hashlib import sha256
from typing import Protocol, TypedDict
from uuid import uuid4

from socialguard_models.api.observability import log_event
from socialguard_models.api.schemas import TranscriptionModel


@dataclass(frozen=True, slots=True)
class SubmissionCommand:
    """Immutable command accepted by the storage and processing layer."""

    caller_id: str
    model: TranscriptionModel
    audio_uri: str
    callback_url: str
    report_id: str
    idempotency_key: str


@dataclass(frozen=True, slots=True)
class IdempotencyConflictError(Exception):
    """A caller reused a key for a submission with different request data."""


@dataclass(frozen=True, slots=True)
class SubmissionUnavailableError(Exception):
    """The gateway could not durably dispatch an accepted submission."""


@dataclass(frozen=True, slots=True)
class SubmissionResult:
    """Immutable result returned immediately after a job is accepted."""

    transcription_id: str


@dataclass(frozen=True, slots=True)
class SubmissionRecord:
    """Bounded ledger record used to replay a caller's idempotent request."""

    transcription_id: str
    fingerprint: str
    state: str
    command: SubmissionCommand


class SubmissionLedger(Protocol):
    """Atomic create-if-absent storage used by the bounded Modal implementation."""

    async def get(self, key: str) -> SubmissionRecord | None:  # noqa: D102
        ...

    async def put_if_absent(self, key: str, record: SubmissionRecord) -> bool:  # noqa: D102
        ...

    async def set_state(self, key: str, state: str) -> None:  # noqa: D102
        ...

    async def claim_dispatch(self, key: str) -> bool:  # noqa: D102
        ...

    async def release_dispatch_claim(self, key: str) -> None:  # noqa: D102
        ...


class SubmissionDispatcher(Protocol):
    """Start CPU orchestration without waiting for inference completion."""

    async def dispatch(  # noqa: D102
        self,
        ledger_key: str,
        transcription_id: str,
        command: SubmissionCommand,
    ) -> None: ...


def _canonical_fingerprint(command: SubmissionCommand) -> str:
    payload = {
        "audio_uri": command.audio_uri,
        "callback_url": command.callback_url,
        "model": command.model.value,
        "report_id": command.report_id,
    }
    return sha256(
        json.dumps(payload, separators=(",", ":"), sort_keys=True).encode()
    ).hexdigest()


def _ledger_key(command: SubmissionCommand) -> str:
    raw = f"{command.caller_id}\0{command.idempotency_key}".encode()
    return f"submission:{sha256(raw).hexdigest()}"


class _CommandLogFields(TypedDict):
    """Safe correlation fields attached to submission lifecycle events."""

    report_id: str
    idempotency_key: str
    model: str
    audio_uri: str


def _command_log_fields(command: SubmissionCommand) -> _CommandLogFields:
    """Return safe identifiers shared by submission lifecycle events."""
    return {
        "report_id": command.report_id,
        "idempotency_key": command.idempotency_key,
        "model": command.model.value,
        "audio_uri": command.audio_uri,
    }


class LedgerSubmitter:
    """Implement idempotency with atomic submission and dispatch claims."""

    def __init__(
        self,
        ledger: SubmissionLedger,
        dispatcher: SubmissionDispatcher,
        *,
        handoff_attempts: int = 20,
        handoff_interval_seconds: float = 0.05,
        sleeper: Callable[[float], Awaitable[None]] = asyncio.sleep,
    ) -> None:
        """Bind dispatch dependencies and bounded concurrent-handoff behavior."""
        if handoff_attempts < 1:
            message = "handoff_attempts must be positive"
            raise ValueError(message)
        if handoff_interval_seconds < 0:
            message = "handoff_interval_seconds must not be negative"
            raise ValueError(message)
        self._ledger = ledger
        self._dispatcher = dispatcher
        self._handoff_attempts = handoff_attempts
        self._handoff_interval_seconds = handoff_interval_seconds
        self._sleeper = sleeper

    async def _get_submission_record(
        self,
        key: str,
        command: SubmissionCommand,
        fingerprint: str,
    ) -> SubmissionRecord:
        """Create a submission once or return the matching original record."""
        new_record = SubmissionRecord(
            transcription_id=f"transcription_{uuid4().hex}",
            fingerprint=fingerprint,
            state="dispatch_pending",
            command=command,
        )
        if await self._ledger.put_if_absent(key, new_record):
            log_event(
                "submission_record_created",
                transcription_id=new_record.transcription_id,
                **_command_log_fields(command),
            )
            return new_record
        existing = await self._ledger.get(key)
        if existing is None:
            log_event(
                "submission_ledger_inconsistent",
                level=logging.ERROR,
                **_command_log_fields(command),
            )
            raise SubmissionUnavailableError
        if existing.fingerprint != fingerprint:
            log_event(
                "submission_idempotency_conflict",
                level=logging.WARNING,
                transcription_id=existing.transcription_id,
                existing_state=existing.state,
                **_command_log_fields(command),
            )
            raise IdempotencyConflictError
        log_event(
            "submission_record_replayed",
            transcription_id=existing.transcription_id,
            state=existing.state,
            **_command_log_fields(command),
        )
        return existing

    async def submit(self, command: SubmissionCommand) -> SubmissionResult:
        """Dispatch work or verify another request dispatched it before returning."""
        key = _ledger_key(command)
        record = await self._get_submission_record(
            key,
            command,
            _canonical_fingerprint(command),
        )

        for attempt in range(self._handoff_attempts):
            if record.state == "dispatched":
                log_event(
                    "submission_already_dispatched",
                    transcription_id=record.transcription_id,
                    **_command_log_fields(command),
                )
                return SubmissionResult(transcription_id=record.transcription_id)
            if record.state != "dispatch_pending":
                log_event(
                    "submission_invalid_state",
                    level=logging.ERROR,
                    transcription_id=record.transcription_id,
                    state=record.state,
                    **_command_log_fields(command),
                )
                raise SubmissionUnavailableError
            if await self._ledger.claim_dispatch(key):
                log_event(
                    "submission_dispatch_claimed",
                    transcription_id=record.transcription_id,
                    **_command_log_fields(command),
                )
                try:
                    await self._dispatcher.dispatch(
                        key,
                        record.transcription_id,
                        record.command,
                    )
                except Exception as exc:
                    log_event(
                        "submission_dispatch_failed",
                        level=logging.ERROR,
                        transcription_id=record.transcription_id,
                        error_type=type(exc).__name__,
                        detail=str(exc),
                        exc_info=True,
                        **_command_log_fields(command),
                    )
                    await self._ledger.release_dispatch_claim(key)
                    raise SubmissionUnavailableError from exc
                # Keep the claim if this non-critical marker write fails: the worker
                # repairs it at startup, avoiding duplicate dispatch after a spawn.
                try:
                    await self._ledger.set_state(key, "dispatched")
                except Exception as exc:  # noqa: BLE001 - worker startup repairs this state.
                    log_event(
                        "submission_state_write_failed",
                        level=logging.WARNING,
                        transcription_id=record.transcription_id,
                        error_type=type(exc).__name__,
                        detail=str(exc),
                        exc_info=True,
                        **_command_log_fields(command),
                    )
                return SubmissionResult(transcription_id=record.transcription_id)
            if attempt < self._handoff_attempts - 1:
                log_event(
                    "submission_dispatch_claim_busy",
                    level=logging.WARNING,
                    transcription_id=record.transcription_id,
                    attempt=attempt + 1,
                    **_command_log_fields(command),
                )
                await self._sleeper(self._handoff_interval_seconds)
                refreshed = await self._ledger.get(key)
                if refreshed is None:
                    log_event(
                        "submission_record_disappeared",
                        level=logging.ERROR,
                        transcription_id=record.transcription_id,
                        **_command_log_fields(command),
                    )
                    raise SubmissionUnavailableError
                record = refreshed

        log_event(
            "submission_dispatch_handoff_exhausted",
            level=logging.ERROR,
            transcription_id=record.transcription_id,
            attempts=self._handoff_attempts,
            **_command_log_fields(command),
        )
        raise SubmissionUnavailableError


class TranscriptionSubmitter(Protocol):
    """Accept transcription commands without exposing implementation details."""

    async def submit(self, command: SubmissionCommand) -> SubmissionResult:  # noqa: D102
        ...
