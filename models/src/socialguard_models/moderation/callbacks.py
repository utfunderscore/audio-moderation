"""Pure mapping of moderation outcomes to the proposed workflow callback payload."""

from typing import TypedDict

from socialguard_models.moderation.contracts import (
    CompletedOutcome,
    ModerationOutcome,
    ModerationScores,
)


class ModerationResult(TypedDict):
    """Moderation counterpart of the transcriptionResult callback object."""

    jobId: str
    moderationTaskId: str
    scores: ModerationScores


def completion_outcome(
    *, job_id: str, moderation_task_id: str, outcome: ModerationOutcome
) -> dict[str, object]:
    """Map moderation output to the same success/failure envelope as transcription."""
    if isinstance(outcome, CompletedOutcome):
        result: ModerationResult = {
            "jobId": job_id,
            "moderationTaskId": moderation_task_id,
            "scores": outcome.scores,
        }
        return {"type": "success", "moderationResult": result}
    return {
        "type": "failure",
        "error": "ModerationFailed",
        "cause": outcome.cause,
    }
