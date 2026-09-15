"""Moderation input dispatch and output mapping for the shared pipeline."""

from dataclasses import dataclass
from typing import ClassVar

from socialguard_models.contracts import FailedOutcome
from socialguard_models.idempotency import ModelFamily
from socialguard_models.model_job import SubmittedModel
from socialguard_models.moderation.callbacks import completion_outcome
from socialguard_models.moderation.contracts import (
    CompletedOutcome,
    ModerationScores,
    ModerationTask,
)
from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS


@dataclass(frozen=True, slots=True)
class ModerationJob:
    """Serializable task data; model instances stay in their GPU runtimes."""

    task: ModerationTask
    family: ClassVar[ModelFamily] = "moderation"

    @property
    def model(self) -> str:
        return self.task.model

    async def submit(self, audio_url: str) -> SubmittedModel[ModerationScores]:
        submitter = MODEL_SUBMITTERS.get(self.task.model)
        if submitter is None:
            raise ValueError(f"unsupported moderation model: {self.task.model}")
        return await submitter(audio_url, self.task.transcription)

    def callback_outcome(
        self, result: ModerationScores | FailedOutcome, task_id: str
    ) -> dict[str, object]:
        return completion_outcome(
            job_id=self.task.pipeline_task_id,
            moderation_task_id=task_id,
            outcome=(
                result if isinstance(result, FailedOutcome)
                else CompletedOutcome(scores=result)
            ),
        )
