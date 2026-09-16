"""Common HTTP input metadata and asynchronous acceptance response."""

from typing import Literal

from pydantic import BaseModel, ConfigDict


class AudioRequest(BaseModel):
    model_config = ConfigDict(
        extra="forbid",
        json_schema_extra={"$schema": "https://json-schema.org/draft/2020-12/schema"},
    )

    audio_uri: str
    idempotency_key: str
    pipeline_task_id: str
    task_token: str


class QueuedResponse(BaseModel):
    """An accepted model task that will complete asynchronously."""

    model_config = ConfigDict(extra="forbid")

    status: Literal["queued"] = "queued"
    task_id: str
