"""Shared CPU deployment: accept HTTP requests and coordinate GPU workers."""

import modal
from fastapi import FastAPI

from socialguard_models.api.transcription_api import router as transcription_router
from socialguard_models.api.moderation_api import router as moderation_router
from socialguard_models.modal_app import app, cpu_image

MAX_CONCURRENT_REQUESTS = 32

@app.function(  # pyright: ignore[reportUnknownMemberType]
    cpu=1.0,
    image=cpu_image,
    timeout=1_000,
    max_containers=1,
)
@modal.concurrent(max_inputs=MAX_CONCURRENT_REQUESTS)  # pyright: ignore[reportUnknownMemberType]
@modal.asgi_app(requires_proxy_auth=True)  # pyright: ignore[reportUnknownMemberType]
def serve_api() -> FastAPI:
    """Serve the API routes in the shared CPU container."""
    api = FastAPI()
    api.include_router(transcription_router, prefix="/transcription", tags=["transcription"])
    api.include_router(moderation_router, prefix="/moderation", tags=["moderation"])
    api.include_router(transcription_router, include_in_schema=False)
    return api
