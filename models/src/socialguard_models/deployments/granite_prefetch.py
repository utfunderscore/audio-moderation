"""CPU-only Modal prefetch for the pinned Granite Speech model."""

import modal

from socialguard_models.deployments.granite_resources import (
    CACHE_DIRECTORY,
    IMAGE,
    MODEL_CACHE,
    MODEL_ID,
    MODEL_REVISION,
)

PREFETCH_APP = modal.App("socialguard-granite-prefetch")


@PREFETCH_APP.function(  # pyright: ignore[reportUnknownMemberType]
    image=IMAGE,
    volumes={CACHE_DIRECTORY: MODEL_CACHE},
    timeout=1_200,
)
def prefetch_model() -> str:
    """Download the immutable Granite snapshot and commit it to the Volume."""
    from huggingface_hub import snapshot_download

    snapshot_path = snapshot_download(
        repo_id=MODEL_ID,
        revision=MODEL_REVISION,
        cache_dir=CACHE_DIRECTORY,
    )
    MODEL_CACHE.commit()
    return snapshot_path
