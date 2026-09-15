"""Sample-accurate windowing for Roblox's short-segment classifier."""

from collections.abc import Iterator

from socialguard_models.audio import SAMPLE_RATE

WINDOW_SAMPLES = 15 * SAMPLE_RATE
STRIDE_SAMPLES = 12 * SAMPLE_RATE
# The reference STFT/mask paths agree on multiples of two 160-sample hops.
# At least eight hops also leave frames after the encoder's time reduction.
ALIGNMENT_SAMPLES = 320
MIN_INFERENCE_SAMPLES = 1_280


def audio_windows(sample_count: int) -> Iterator[tuple[int, int]]:
    """Cover every sample, using an end-anchored full window for the final tail."""
    if sample_count <= 0:
        raise ValueError("audio must contain at least one sample")
    if sample_count <= WINDOW_SAMPLES:
        yield 0, sample_count
        return
    final_start = sample_count - WINDOW_SAMPLES
    for start in range(0, final_start + 1, STRIDE_SAMPLES):
        yield start, start + WINDOW_SAMPLES
    if final_start % STRIDE_SAMPLES:
        yield final_start, sample_count


def pad_pcm_window(pcm: bytes) -> bytes:
    """Append silence for short/unaligned PCM16 input; never exceed 15 seconds.

    Padding is treated as audio by the reference all-valid mask. It is at most
    319 samples for ordinary clips, or up to 80 ms for extremely short clips.
    """
    if not pcm or len(pcm) % 2:
        raise ValueError("expected nonempty PCM16 samples")
    samples = len(pcm) // 2
    if samples > WINDOW_SAMPLES:
        raise ValueError("audio window exceeds 15 seconds")
    padded_samples = max(
        MIN_INFERENCE_SAMPLES,
        (samples + ALIGNMENT_SAMPLES - 1) // ALIGNMENT_SAMPLES * ALIGNMENT_SAMPLES,
    )
    return pcm + bytes((padded_samples - samples) * 2)
