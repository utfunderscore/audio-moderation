"""Run Roblox moderation directly, bypassing HTTP, scheduling, and callbacks.

    uv run modal run scripts/moderate.py --audio-file ./sample.wav
    uv run modal run scripts/moderate.py --audio-url https://example.com/sample.wav
"""

import json
import sys
from pathlib import Path
from time import perf_counter

from socialguard_models.modal_app import app
from socialguard_models.moderation.models.roblox_voice_safety import (
    MAX_AUDIO_BYTES,
    PUBLIC_MODEL_ID,
    RobloxVoiceSafety,
)


@app.local_entrypoint(name="moderate")  # pyright: ignore[reportUnknownMemberType]
def main(audio_file: str = "", audio_url: str = "") -> None:
    """Moderate one audio input and print the five category scores as JSON."""
    if sum(bool(value) for value in (audio_file, audio_url)) != 1:
        raise SystemExit("provide exactly one of --audio-file or --audio-url")
    started = perf_counter()
    if audio_file:
        path = Path(audio_file)
        if not path.is_file():
            raise SystemExit(f"audio file not found: {path}")
        if path.stat().st_size > MAX_AUDIO_BYTES:
            raise ValueError("audio file exceeds the 512 MiB limit")
        scores = RobloxVoiceSafety().moderate_bytes.remote(path.read_bytes())  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    else:
        scores = RobloxVoiceSafety().moderate.remote(audio_url, "")  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    print(json.dumps(scores, allow_nan=False))
    print(f"\n[{PUBLIC_MODEL_ID} moderated in {perf_counter() - started:.1f}s]", file=sys.stderr)
