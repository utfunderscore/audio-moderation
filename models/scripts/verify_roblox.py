"""GPU regression smoke test and synthetic-audio benchmark for the pinned runtime.

    uv run modal run scripts/verify_roblox.py

This exercises real weights, preprocessing, and windowing. It does not measure
moderation quality; use representative labeled audio to choose policy thresholds.
"""

import io
import json
import resource
import wave
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter

from socialguard_models.modal_app import app
from socialguard_models.moderation.models.roblox_voice_safety import (
    MODEL_CACHE_PATH,
    MODEL_REVISION,
    RobloxVoiceSafety,
    model_cache,
    roblox_image,
)
from socialguard_models.moderation.models.roblox_voice_safety_scores import map_scores


def wav_bytes(pcm: bytes) -> bytes:
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(16_000)
        output.writeframes(pcm)
    return buffer.getvalue()


@app.function(
    image=roblox_image,
    gpu="L4",
    memory=16_384,
    timeout=660,
    volumes={MODEL_CACHE_PATH: model_cache},
)
def verify() -> dict[str, object]:
    import numpy as np
    import torch

    # Exercise the deployed class implementation without spawning a second GPU.
    worker = RobloxVoiceSafety._get_user_cls()()
    started = perf_counter()
    worker.load_model()
    load_seconds = perf_counter() - started
    torch.cuda.reset_peak_memory_stats()

    lengths = [1, 200, 1_279, 1_280, 1_281, 16_001, 239_999, 240_000, 240_001, 480_123]
    for samples in lengths:
        scores = worker.moderate_bytes(wav_bytes(bytes(samples * 2)))
        assert all(0 <= value <= 1 for value in scores.values()), samples

    # Independent WAV loader versus the adapter's PCM-to-float conversion.
    pcm = np.random.default_rng(0).integers(-8_000, 8_000, 240_000, dtype=np.int16).tobytes()
    with TemporaryDirectory() as directory:
        path = Path(directory, "reference.wav")
        path.write_bytes(wav_bytes(pcm))
        waveform = worker._reference.load_audio(path)
        raw = worker._reference.run_inference(worker._model, waveform)
        reference = map_scores(worker._labels, raw["probs"][0].tolist())
    actual = worker.moderate_bytes(wav_bytes(pcm))
    maximum_error = max(abs(actual[key] - reference[key]) for key in actual)
    assert maximum_error <= 1e-6, maximum_error

    timings = {}
    for seconds in (15, 300):
        started = perf_counter()
        worker.moderate_bytes(wav_bytes(bytes(seconds * 16_000 * 2)))
        timings[str(seconds)] = perf_counter() - started
    assert load_seconds + timings["300"] < 600, "insufficient margin in the 660-second budget"
    return {
        "revision": MODEL_REVISION,
        "device": torch.cuda.get_device_name(),
        "load_seconds": load_seconds,
        "tested_sample_lengths": lengths,
        "upstream_maximum_absolute_error": maximum_error,
        "warm_recording_seconds": timings,
        "peak_cuda_allocated_bytes": torch.cuda.max_memory_allocated(),
        "peak_host_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024,
    }


@app.local_entrypoint()
def main() -> None:
    started = perf_counter()
    result = verify.remote()
    result["remote_check_elapsed_seconds"] = perf_counter() - started
    print(json.dumps(result, indent=2, allow_nan=False))
