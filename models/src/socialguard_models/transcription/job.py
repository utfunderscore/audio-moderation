"""Transcription input dispatch and output mapping for the shared pipeline."""

from dataclasses import dataclass
from typing import ClassVar, cast

from socialguard_models.contracts import FailedOutcome
from socialguard_models.idempotency import ModelFamily
from socialguard_models.model_job import SubmittedModel
from socialguard_models.transcription.callbacks import completion_outcome
from socialguard_models.transcription.contracts import CompletedOutcome, ModelType, TranscriptionTask
from socialguard_models.transcription.models.granite import GraniteSpeech


@dataclass(frozen=True, slots=True)
class TranscriptionJob:
    task: TranscriptionTask
    family: ClassVar[ModelFamily] = "transcription"
    callback_environment_variable: ClassVar[str] = "TRANSCRIPTION_CALLBACK_URI"

    @property
    def model(self) -> str:
        return self.task.model

    async def submit(self, audio_url: str) -> SubmittedModel[str]:
        match self.task.model:
            case ModelType.GRANITE:
                return cast(
                    SubmittedModel[str],
                    await GraniteSpeech().transcribe.spawn.aio(audio_url),  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
                )
            case _:  # pyright: ignore[reportUnnecessaryComparison]
                raise ValueError(f"unsupported transcription model: {self.task.model}")

    def callback_outcome(
        self, result: str | FailedOutcome, task_id: str
    ) -> dict[str, object]:
        return completion_outcome(
            job_id=self.task.pipeline_task_id,
            asr_task_id=task_id,
            outcome=result if isinstance(result, FailedOutcome) else CompletedOutcome(text=result),
        )
