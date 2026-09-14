"""The family-specific boundary of the shared model execution pipeline."""

from typing import Awaitable, ClassVar, Protocol

from socialguard_models.contracts import AudioTask, FailedOutcome
from socialguard_models.idempotency import ModelFamily


class AsyncResult[Result](Protocol):
    def aio(self, timeout: float | None = None) -> Awaitable[Result]: ...


class AsyncCancellation(Protocol):
    def aio(self) -> Awaitable[None]: ...


class SubmittedModel[Result](Protocol):
    """A submitted GPU call with asynchronous result retrieval and cancellation."""

    @property
    def get(self) -> AsyncResult[Result]: ...

    @property
    def cancel(self) -> AsyncCancellation: ...


class ModelJob[Result](Protocol):
    """A serializable task adapter: inputs, GPU dispatch, and output mapping only.

    Implementations must be module-level classes carrying serializable task data,
    not live model instances, credentials, or clients. The shared CPU worker calls
    submit after resolving audio access; inference stays in the GPU runtime.
    """

    @property
    def task(self) -> AudioTask: ...

    family: ClassVar[ModelFamily]

    @property
    def model(self) -> str: ...

    callback_environment_variable: ClassVar[str]

    async def submit(self, audio_url: str) -> SubmittedModel[Result]: ...

    def callback_outcome(
        self, result: Result | FailedOutcome, task_id: str
    ) -> dict[str, object]: ...
