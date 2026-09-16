"""Pure mapping of transcription outcomes to workflow callback payloads."""
from socialguard_models.transcription.contracts import CompletedOutcome, TranscriptionOutcome


def completion_outcome(
    *, job_id: str, asr_task_id: str, outcome: TranscriptionOutcome
) -> dict[str, object]:
    """Map transcription output to its workflow callback contract."""
    if isinstance(outcome, CompletedOutcome):
        payload: dict[str, object] = {
            "type": "success",
            "transcriptionResult": {
                "jobId": job_id,
                "asrTaskId": asr_task_id,
                "transcription": outcome.text,
            },
        }
    else:
        payload = {
            "type": "failure",
            "error": "TranscriptionFailed",
            "cause": outcome.cause,
        }
    return payload
