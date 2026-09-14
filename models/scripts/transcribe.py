"""Developer tooling to exercise transcription models without the HTTP API.

This bypasses the FastAPI endpoint, the Modal Dict idempotency layer, S3
presigning, and completion callbacks so a model can be tested on its own.

    uv run modal run scripts/transcribe.py --audio-file ./sample.wav
    uv run modal run scripts/transcribe.py --audio-url https://example.com/sample.wav

`modal run` creates an ephemeral App from these local definitions, using the
current GPU image definition and shared model cache volume.
"""

from __future__ import annotations

import sys
from pathlib import Path
from time import perf_counter

from socialguard_models.transcription.models.granite import MAX_AUDIO_BYTES, GraniteSpeech
from socialguard_models.modal_app import app


@app.local_entrypoint()  # pyright: ignore[reportUnknownMemberType]
def main(audio_file: str = "", audio_url: str = "") -> None:
    """Transcribe one audio input and print the transcript."""
    sources = [value for value in (audio_file, audio_url) if value]
    if len(sources) != 1:
        raise SystemExit("provide exactly one of --audio-file or --audio-url")

    started = perf_counter()
    if audio_file:
        path = Path(audio_file)
        if not path.is_file():
            raise SystemExit(f"audio file not found: {path}")
        if path.stat().st_size > MAX_AUDIO_BYTES:
            raise ValueError("audio file exceeds the 512 MiB limit")
        transcript = GraniteSpeech().transcribe_bytes.remote(path.read_bytes())  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    else:
        transcript = GraniteSpeech().transcribe.remote(audio_url)  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    elapsed = perf_counter() - started

    print(transcript)
    print(f"\n[granite transcribed in {elapsed:.1f}s]", file=sys.stderr)
